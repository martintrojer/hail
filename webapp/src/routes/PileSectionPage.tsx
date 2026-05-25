import { useQueryClient } from '@tanstack/react-query';
import { Link } from '@tanstack/react-router';
import { type ComponentType, type FormEvent, type ReactNode, useState } from 'react';
import type { HailApiClient, PileItem, PileViewResponse } from '../api/client';
import { useClassifyThreadMutation, useReplyLaterView, useSetAsideView } from '../api/query';
import { defaultApiClient } from '../api/query';
import { queryKeys } from '../api/queryKeys';
import { ErrorState } from '../components/ErrorState';
import { Bookmark, Clock, Send, iconSizeProps } from '../components/icons';
import { LoadingState } from '../components/LoadingState';
import { useOptionalUndoToast } from '../components/UndoToastProvider';
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
  inlineReply?: boolean;
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
    inlineReply: true,
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

function InlineReplyInput({ threadId, client, onSent }: { threadId: string; client: HailApiClient; onSent: () => void }) {
  const [expanded, setExpanded] = useState(false);
  const [body, setBody] = useState('');
  const [sending, setSending] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function handleSubmit(event: FormEvent) {
    event.preventDefault();
    if (!body.trim() || sending) return;
    setSending(true);
    setError(null);
    try {
      await client.sendReply(threadId, { body_markdown: body.trim() });
      setBody('');
      setExpanded(false);
      onSent();
    } catch {
      setError('Reply failed. Try again.');
    } finally {
      setSending(false);
    }
  }

  if (!expanded) {
    return (
      <button
        type="button"
        onClick={(e) => { e.preventDefault(); e.stopPropagation(); setExpanded(true); }}
        className="mt-2 text-sm font-semibold text-accent-blue outline-none hover:underline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent-blue"
      >
        Quick reply…
      </button>
    );
  }

  return (
    <form onSubmit={(e) => void handleSubmit(e)} className="mt-3" onClick={(e) => e.preventDefault()}>
      <textarea
        value={body}
        onChange={(e) => setBody(e.target.value)}
        placeholder="Type your reply…"
        rows={3}
        disabled={sending}
        className="w-full rounded-md border border-border-hairline bg-bg-canvas px-3 py-2 text-sm text-ink-primary outline-none placeholder:text-ink-tertiary focus-visible:border-accent-blue focus-visible:ring-1 focus-visible:ring-accent-blue disabled:opacity-60"
        autoFocus
      />
      <div className="mt-2 flex items-center gap-2">
        <button
          type="submit"
          disabled={!body.trim() || sending}
          className="inline-flex items-center gap-1.5 rounded-full bg-accent-blue px-3 py-1 text-xs font-semibold text-white outline-none hover:bg-accent-blue-hover focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent-blue disabled:opacity-60"
        >
          <Send {...iconSizeProps.sm} aria-hidden="true" />
          {sending ? 'Sending…' : 'Send'}
        </button>
        <button
          type="button"
          onClick={() => { setExpanded(false); setBody(''); setError(null); }}
          disabled={sending}
          className="text-xs font-semibold text-ink-secondary outline-none hover:text-ink-primary"
        >
          Cancel
        </button>
      </div>
      {error ? <p className="mt-1 text-xs text-accent-red">{error}</p> : null}
    </form>
  );
}

function PileRow({
  item,
  config,
  client,
}: {
  item: PileItem;
  config: SectionConfig;
  client?: HailApiClient;
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
        {config.inlineReply && client ? (
          <InlineReplyInput
            threadId={item.thread_id}
            client={client}
            onSent={() => {
              void queryClient.invalidateQueries({ queryKey: queryKeys.views() });
            }}
          />
        ) : null}
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

function PileList({ query, config, client }: { query: ReturnType<SectionConfig['useView']>; config: SectionConfig; client?: HailApiClient }) {
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
        <PileRow key={item.thread_id} item={item} config={config} client={client} />
      ))}
    </div>
  );
}

export function PileSectionPage({ kind }: PileSectionPageProps) {
  const config = configs[kind];
  const query = config.useView();
  const client = defaultApiClient;
  const undoToast = useOptionalUndoToast();
  void undoToast; // available for future use

  return (
    <AppShell
      title={config.title}
      description={config.description}
      list={<PileList query={query} config={config} client={client} />}
    />
  );
}
