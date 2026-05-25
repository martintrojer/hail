import { useQueryClient } from '@tanstack/react-query';
import { Link } from '@tanstack/react-router';
import { type FormEvent, useState } from 'react';
import type { ComposeRequest, HailApiClient, PileItem, PileViewResponse } from '../api/client';
import {
  useClassifyThreadMutation,
  useReplyLaterView,
  useSendComposeMutation,
  useSetAsideView,
} from '../api/query';
import { defaultApiClient } from '../api/query';
import { queryKeys } from '../api/queryKeys';
import { ErrorState } from '../components/ErrorState';
import { Send, iconSizeProps } from '../components/icons';
import { MailRow } from '../components/MailRow';
import { LoadingState } from '../components/LoadingState';
import { StateCard } from '../components/StateCard';
import { ListView } from '../components/ListView';
import { AppShell } from '../layout/AppShell';
import { pilePreview } from '../lib/pilePreview';

interface PileSectionPageProps {
  kind: 'set-aside' | 'reply-later';
}

interface SectionConfig {
  title: string;
  description: string;
  emptyTitle: string;
  emptyBody: string;
  actionLabel: string;
  useView: () => ReturnType<typeof useSetAsideView> | ReturnType<typeof useReplyLaterView>;
}

const configs: Record<PileSectionPageProps['kind'], SectionConfig> = {
  'set-aside': {
    title: 'Set Aside',
    description: 'Threads you want nearby but not in the Imbox wait here.',
    emptyTitle: 'Nothing set aside.',
    emptyBody: 'Set threads aside when you want to come back to them.',
    actionLabel: 'Move back to Imbox',
    useView: useSetAsideView,
  },
  'reply-later': {
    title: 'Reply Later',
    description: 'Mail that needs a response can wait in this pile.',
    emptyTitle: 'Nothing to reply to later.',
    emptyBody: 'Mark threads for reply when you are ready.',
    actionLabel: 'Move back to Imbox',
    useView: useReplyLaterView,
  },
};

// ---------------------------------------------------------------------------
// Reply panel (right column in Reply Later view)
// ---------------------------------------------------------------------------

function ReplyPanel({
  item,
  client,
  onSent,
}: {
  item: PileItem;
  client: HailApiClient;
  onSent: () => void;
}) {
  const preview = pilePreview(item);
  const [body, setBody] = useState('');
  const [error, setError] = useState<string | null>(null);
  const [sent, setSent] = useState(false);
  const sendReply = useSendComposeMutation(client);
  const moveBack = useClassifyThreadMutation(client);

  async function handleSubmit(event: FormEvent) {
    event.preventDefault();
    if (!body.trim() || sending) return;
    setError(null);
    const request: ComposeRequest = {
      to: [],
      cc: [],
      bcc: [],
      subject: '',
      body_markdown: body.trim(),
      attachments: [],
    };
    try {
      await sendReply.mutateAsync({ threadId: item.thread_id, request });
      try {
        await moveBack.mutateAsync({ threadId: item.thread_id, to: 'imbox' });
      } catch {
        setError('Reply sent, but moving it back to Imbox failed. Try moving it manually.');
        onSent();
        return;
      }
      setBody('');
      setSent(true);
      onSent();
    } catch {
      setError('Reply failed. Try again.');
    }
  }

  const sending = sendReply.isPending || moveBack.isPending;

  return (
    <div className="flex h-full flex-col">
      {/* Thread context */}
      <div className="border-b border-border-hairline px-4 py-4">
        <p className="text-sm font-semibold text-ink-primary">{preview.sender}</p>
        <p className="mt-1 text-sm text-ink-secondary">{preview.subject}</p>
        <p className="mt-2 text-sm leading-relaxed text-ink-tertiary">
          {preview.snippet || 'No preview available.'}
        </p>
      </div>

      {/* Reply input */}
      <form onSubmit={(e) => void handleSubmit(e)} className="flex flex-1 flex-col px-4 py-4">
        {sent ? (
          <div className="flex flex-1 items-center justify-center">
            <p className="text-sm font-semibold text-ink-secondary">Reply sent ✓</p>
          </div>
        ) : (
          <>
            <textarea
              data-hail-reply-box="true"
              value={body}
              onChange={(e) => setBody(e.target.value)}
              placeholder="Write your reply…"
              disabled={sending}
              className="flex-1 resize-none rounded-md border border-border-hairline bg-bg-canvas px-3 py-3 text-sm leading-relaxed text-ink-primary outline-none placeholder:text-ink-tertiary focus-visible:border-accent-blue focus-visible:ring-1 focus-visible:ring-accent-blue disabled:opacity-60"
            />
            <div className="mt-3 flex items-center justify-between">
              <button
                type="submit"
                disabled={!body.trim() || sending}
                className="inline-flex items-center gap-1.5 rounded-full bg-accent-blue px-4 py-1.5 text-sm font-semibold text-white focus-ring outline-none hover:bg-accent-blue-hover disabled:opacity-60"
              >
                <Send {...iconSizeProps.sm} aria-hidden="true" />
                {sending ? (moveBack.isPending ? 'Moving…' : 'Sending…') : 'Reply'}
              </button>
              {error ? <p className="text-xs text-accent-red">{error}</p> : null}
            </div>
          </>
        )}
      </form>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Pile row (shared by Set Aside and Reply Later)
// ---------------------------------------------------------------------------

function PileRow({
  item,
  config,
  kind,
  selected,
  onSelect,
}: {
  item: PileItem;
  config: SectionConfig;
  kind: string;
  selected?: boolean;
  onSelect?: () => void;
}) {
  const preview = pilePreview(item);
  const queryClient = useQueryClient();
  const moveBack = useClassifyThreadMutation(undefined, {
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: queryKeys.views() });
    },
    onError: () => {
      // Still refresh the list even on error so stale rows are cleared.
      void queryClient.invalidateQueries({ queryKey: queryKeys.views() });
    },
  });

  const rowContent = (
    <MailRow
      from={preview.sender}
      subject={preview.subject}
      preview={preview.snippet || 'No preview available.'}
      receivedAt={item.added_at}
    />
  );

  return (
    <div
      className={`group flex items-stretch gap-3 border-b border-border-hairline hover:bg-bg-hover focus-within:bg-bg-selected ${selected ? 'bg-bg-selected' : ''}`}
    >
      {onSelect ? (
        // Reply Later: clicking selects the row to show reply panel
        <button
          type="button"
          onClick={onSelect}
          className="block min-w-0 flex-1 border-l-[3px] border-l-transparent py-4 pl-3 text-left outline-none focus-visible:border-l-accent-blue focus-visible:outline-none sm:py-5"
          aria-label={`Select ${preview.subject} from ${preview.sender} to reply`}
          aria-pressed={selected}
        >
          {rowContent}
        </button>
      ) : (
        // Set Aside: clicking navigates to thread
        <Link
          to="/thread/$threadId"
          params={{ threadId: item.thread_id }}
          search={{ from: kind }}
          className="block min-w-0 flex-1 border-l-[3px] border-l-transparent py-4 pl-3 outline-none focus-visible:border-l-accent-blue focus-visible:outline-none sm:py-5"
          aria-label={`Open ${preview.subject} from ${preview.sender}`}
          data-hail-mail-list-item="true"
        >
          {rowContent}
        </Link>
      )}
      <div className="flex shrink-0 items-center pr-1 sm:pr-0">
        <button
          type="button"
          className="rounded-full border border-border-menu px-3 py-1 text-xs font-semibold text-ink-secondary opacity-90 focus-ring outline-none hover:bg-bg-selected hover:text-accent-blue sm:opacity-0 sm:group-hover:opacity-100 sm:group-focus-within:opacity-100"
          onClick={() => moveBack.mutate({ threadId: item.thread_id, to: 'imbox' })}
          disabled={moveBack.isPending}
        >
          {moveBack.isPending ? 'Moving…' : config.actionLabel}
        </button>
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Reply Later: two-column layout (list left, reply right)
// ---------------------------------------------------------------------------

function ReplyLaterList({
  query,
  config,
  client,
}: {
  query: ReturnType<SectionConfig['useView']>;
  config: SectionConfig;
  client: HailApiClient;
}) {
  const queryClient = useQueryClient();
  const [selectedId, setSelectedId] = useState<string | null>(null);

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

  const selectedItem = data.items.find((i) => i.thread_id === selectedId) ?? null;

  return (
    <div className="flex min-h-[400px] gap-0 sm:gap-0">
      {/* Left: thread list */}
      <div className={`min-w-0 ${selectedItem ? 'w-1/2 border-r border-border-hairline' : 'w-full'}`}>
        <ListView
          items={data.items}
          renderItem={(item) => (
            <PileRow
              item={item}
              config={config}
              kind="reply-later"
              selected={item.thread_id === selectedId}
              onSelect={() => setSelectedId(item.thread_id === selectedId ? null : item.thread_id)}
            />
          )}
          keyExtractor={(item) => item.thread_id}
          hasMore={false}
          isLoadingMore={false}
          onLoadMore={() => {}}
          emptyState={<StateCard title={config.emptyTitle} body={config.emptyBody} />}
        />
      </div>

      {/* Right: reply panel */}
      {selectedItem ? (
        <div className="w-1/2">
          <ReplyPanel
            key={selectedItem.thread_id}
            item={selectedItem}
            client={client}
            onSent={() => {
              void queryClient.invalidateQueries({ queryKey: queryKeys.views() });
            }}
          />
        </div>
      ) : null}
    </div>
  );
}

// ---------------------------------------------------------------------------
// Standard pile list (Set Aside)
// ---------------------------------------------------------------------------

function PileList({
  query,
  config,
  kind,
}: {
  query: ReturnType<SectionConfig['useView']>;
  config: SectionConfig;
  kind: PileSectionPageProps['kind'];
}) {
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
    <ListView
      items={data.items}
      renderItem={(item) => <PileRow item={item} config={config} kind={kind} />}
      keyExtractor={(item) => item.thread_id}
      hasMore={false}
      isLoadingMore={false}
      onLoadMore={() => {}}
      emptyState={<StateCard title={config.emptyTitle} body={config.emptyBody} />}
    />
  );
}

// ---------------------------------------------------------------------------
// Page
// ---------------------------------------------------------------------------

export function PileSectionPage({ kind }: PileSectionPageProps) {
  const config = configs[kind];
  const query = config.useView();
  const client = defaultApiClient;

  const list =
    kind === 'reply-later' ? (
      <ReplyLaterList query={query} config={config} client={client} />
    ) : (
      <PileList query={query} config={config} kind={kind} />
    );

  return (
    <AppShell
      title={config.title}
      description={config.description}
      list={list}
    />
  );
}
