import { useQueryClient } from '@tanstack/react-query';
import { Link } from '@tanstack/react-router';
import type { ComponentType, ReactNode } from 'react';
import type { PileItem, PileViewResponse } from '../api/client';
import { useClassifyThreadMutation, useReplyLaterView, useSetAsideView } from '../api/query';
import { queryKeys } from '../api/queryKeys';
import { ErrorState } from '../components/ErrorState';
import { Bookmark, Clock, iconSizeProps } from '../components/icons';
import { LoadingState } from '../components/LoadingState';
import { AppShell } from '../layout/AppShell';
import { formatPileDate, pilePreview } from '../lib/pilePreview';

interface PileSectionPageProps {
  kind: 'set-aside' | 'reply-later';
}

interface SectionConfig {
  title: string;
  description: string;
  emptyTitle: string;
  emptyBody: string;
  actionLabel: string;
  Icon: ComponentType<{ className?: string; size?: number; strokeWidth?: number }>;
  useView: () => ReturnType<typeof useSetAsideView> | ReturnType<typeof useReplyLaterView>;
  meta: (item: PileItem) => ReactNode;
}

const configs: Record<PileSectionPageProps['kind'], SectionConfig> = {
  'set-aside': {
    title: 'Set Aside',
    description: 'Threads you want nearby but not in the Imbox wait here.',
    emptyTitle: 'Nothing set aside.',
    emptyBody: 'Set threads aside when you want to come back to them.',
    actionLabel: 'Move back to Imbox',
    Icon: Bookmark,
    useView: useSetAsideView,
    meta: (item) => <span>Set aside {formatPileDate(item.added_at)}</span>,
  },
  'reply-later': {
    title: 'Reply Later',
    description: 'Mail that needs a response can wait in this pile.',
    emptyTitle: 'Nothing to reply to later.',
    emptyBody: 'Mark threads for reply when you are ready.',
    actionLabel: 'Move back to Imbox',
    Icon: Clock,
    useView: useReplyLaterView,
    meta: (item) => <time dateTime={item.added_at}>Deferred {formatPileDate(item.added_at)}</time>,
  },
};

function StateCard({ title, body }: { title: string; body: string }) {
  return (
    <div className="flex min-h-[300px] flex-col items-center justify-center p-8 text-center">
      <p className="text-lg font-semibold text-ink-primary">{title}</p>
      <p className="mt-2 max-w-sm text-sm leading-6 text-ink-secondary">{body}</p>
    </div>
  );
}

function PileRow({
  item,
  config,
}: {
  item: PileItem;
  config: SectionConfig;
}) {
  const preview = pilePreview(item);
  const queryClient = useQueryClient();
  const moveBack = useClassifyThreadMutation(undefined, {
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: queryKeys.views() });
    },
  });

  return (
    <div className="group flex items-stretch gap-3 border-b border-border-hairline hover:bg-bg-hover focus-within:bg-bg-selected">
      <Link
        to="/thread/$threadId"
        params={{ threadId: item.thread_id }}
        className="block min-w-0 flex-1 border-l-[3px] border-l-transparent py-4 pl-3 outline-none focus-visible:border-l-accent-blue focus-visible:outline-none sm:py-5"
        aria-label={`Open ${preview.subject} from ${preview.sender}`}
        data-hail-mail-list-item="true"
      >
        <div className="flex items-baseline justify-between gap-4">
          <div className="flex min-w-0 items-center gap-2">
            <p className="truncate text-base font-semibold leading-snug text-ink-primary">
              {preview.sender}
            </p>
            <config.Icon className="shrink-0 text-ink-tertiary" {...iconSizeProps.sm} />
          </div>
          <span className="shrink-0 text-sm leading-snug text-ink-tertiary">
            {config.meta(item)}
          </span>
        </div>
        <p className="mt-1 truncate text-[0.95rem] font-normal leading-snug text-ink-secondary">
          {preview.subject}
        </p>
        <p className="mt-1 truncate text-sm font-normal leading-snug text-ink-tertiary">
          {preview.snippet || 'No preview available.'}
        </p>
      </Link>
      <div className="flex shrink-0 items-center pr-1 sm:pr-0">
        <button
          type="button"
          className="rounded-md px-2 py-1.5 text-xs font-semibold text-ink-secondary opacity-90 outline-none hover:bg-bg-selected hover:text-accent-blue focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent-blue sm:opacity-0 sm:group-hover:opacity-100 sm:group-focus-within:opacity-100"
          onClick={() => moveBack.mutate({ threadId: item.thread_id, to: 'imbox' })}
          disabled={moveBack.isPending}
        >
          {moveBack.isPending ? 'Moving…' : config.actionLabel}
        </button>
      </div>
    </div>
  );
}

function PileList({ query, config }: { query: ReturnType<SectionConfig['useView']>; config: SectionConfig }) {
  if (query.isPending) {
    return <LoadingState />;
  }

  if (query.isError) {
    return (
      <ErrorState
        message="This pile failed to load. Refresh and try again."
        onRetry={() => void query.refetch()}
      />
    );
  }

  const data = query.data as PileViewResponse;
  if (data.items.length === 0) {
    return <StateCard title={config.emptyTitle} body={config.emptyBody} />;
  }

  return (
    <div>
      {data.items.map((item) => (
        <PileRow key={item.thread_id} item={item} config={config} />
      ))}
    </div>
  );
}

export function PileSectionPage({ kind }: PileSectionPageProps) {
  const config = configs[kind];
  const query = config.useView();

  return (
    <AppShell
      title={config.title}
      description={config.description}
      list={<PileList query={query} config={config} />}
    />
  );
}
