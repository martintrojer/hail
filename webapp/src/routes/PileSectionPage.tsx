import { useQueryClient } from '@tanstack/react-query';
import { Link } from '@tanstack/react-router';
import { type FormEvent, type MouseEvent, useState } from 'react';
import type { ComposeRequest, HailApiClient, PileItem, PileViewResponse } from '../api/client';
import { useApiClient } from '../api/ApiClientProvider';
import {
  useClassifyThreadMutation,
  useReplyLaterView,
  useSendComposeMutation,
  useSetAsideView,
} from '../api/query';
import { queryKeys } from '../api/queryKeys';
import { ActionableList } from '../components/ActionableList';
import { ErrorState } from '../components/ErrorState';
import { Send } from '../components/icons';
import { MailRow } from '../components/MailRow';
import { LoadingState } from '../components/LoadingState';
import { StateCard } from '../components/StateCard';
import { AppShell } from '../layout/AppShell';
import { pilePreview } from '../lib/pilePreview';
import { plaintextToBodyHtml } from '../lib/plaintextToBodyHtml';
import { cn } from '../lib/utils';
import { Button } from '../components/ui/button';
import { Card, CardContent, CardHeader, CardTitle } from '../components/ui/card';
import { Checkbox } from '../components/ui/checkbox';
import { Textarea } from '../components/ui/textarea';

interface PileSectionPageProps {
  kind: 'set-aside' | 'reply-later';
}

interface SectionConfig {
  title: string;
  description: string;
  emptyTitle: string;
  emptyBody: string;
  actionLabel: string;
  useView: (client: HailApiClient) => ReturnType<typeof useSetAsideView> | ReturnType<typeof useReplyLaterView>;
}

const configs: Record<PileSectionPageProps['kind'], SectionConfig> = {
  'set-aside': {
    title: 'Set Aside',
    description: 'Threads you want nearby but not in the Imbox wait here.',
    emptyTitle: 'Nothing set aside.',
    emptyBody: 'Set threads aside when you want to come back to them.',
    actionLabel: 'Move back to Imbox',
    useView: (client) => useSetAsideView(client),
  },
  'reply-later': {
    title: 'Reply Later',
    description: 'Mail that needs a response can wait in this pile.',
    emptyTitle: 'Nothing to reply to later.',
    emptyBody: 'Mark threads for reply when you are ready.',
    actionLabel: 'Move back to Imbox',
    useView: (client) => useReplyLaterView(client),
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
      body_html: plaintextToBodyHtml(body),
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
    <Card className="h-full rounded-none border-0 ring-0" size="sm">
      {/* Thread context */}
      <CardHeader className="border-b">
        <CardTitle className="text-sm">{preview.sender}</CardTitle>
        <p className="text-sm text-muted-foreground">{preview.subject}</p>
        <p className="text-sm leading-relaxed text-muted-foreground">
          {preview.snippet || 'No preview available.'}
        </p>
      </CardHeader>

      {/* Reply input */}
      <CardContent className="flex flex-1">
        <form onSubmit={(e) => void handleSubmit(e)} className="flex flex-1 flex-col py-4">
        {sent ? (
          <div className="flex flex-1 items-center justify-center">
            <p className="text-sm font-semibold text-muted-foreground">Reply sent ✓</p>
          </div>
        ) : (
          <>
            <Textarea
              data-hail-reply-box="true"
              value={body}
              onChange={(e) => setBody(e.target.value)}
              placeholder="Write your reply…"
              disabled={sending}
              className="flex-1 resize-none"
            />
            <div className="mt-3 flex items-center justify-between gap-3">
              <Button
                type="submit"
                disabled={!body.trim() || sending}
                size="sm"
              >
                <Send data-icon="inline-start" aria-hidden="true" />
                {sending ? (moveBack.isPending ? 'Moving…' : 'Sending…') : 'Reply'}
              </Button>
              {error ? <p role="alert" className="text-xs text-destructive">{error}</p> : null}
            </div>
          </>
        )}
        </form>
      </CardContent>
    </Card>
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
  active,
  onSelect,
  onToggleSelect,
  client,
}: {
  item: PileItem;
  config: SectionConfig;
  kind: string;
  selected?: boolean;
  active?: boolean;
  onSelect?: () => void;
  onToggleSelect?: () => void;
  client: HailApiClient;
}) {
  const preview = pilePreview(item);
  const queryClient = useQueryClient();
  const moveBack = useClassifyThreadMutation(client, {
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: queryKeys.views() });
    },
    onError: () => {
      // Still refresh the list even on error so stale rows are cleared.
      void queryClient.invalidateQueries({ queryKey: queryKeys.views() });
    },
  });

  function handleCheckboxClick(event: MouseEvent<HTMLButtonElement>) {
    event.preventDefault();
    event.stopPropagation();
    onToggleSelect?.();
  }

  const rowContent = (
    <MailRow
      from={preview.sender}
      subject={preview.subject}
      preview={preview.snippet || 'No preview available.'}
      receivedAt={item.added_at}
    />
  );

  const rowClassName = cn(
    'block min-w-0 flex-1 border-l-[3px] border-l-transparent py-4 pl-3 outline-none focus-visible:border-l-primary focus-visible:outline-none sm:py-5',
    onSelect && 'text-left',
  );

  return (
    <div
      className={cn(
        'group flex items-stretch gap-3 border-b border-border hover:bg-muted/50 focus-within:bg-muted/50',
        (active || selected) && 'bg-muted',
      )}
    >
      {onToggleSelect ? (
        <div className="flex shrink-0 items-start pl-3 pt-6 sm:pt-7">
          <Checkbox
            checked={selected}
            onClick={handleCheckboxClick}
            aria-label={`${selected ? 'Deselect' : 'Select'} ${preview.sender || 'Unknown sender'}`}
          />
        </div>
      ) : null}
      {onSelect ? (
        // Reply Later: clicking selects the row to show reply panel
          <Button
            type="button"
            variant="ghost"
            onClick={onSelect}
            className={cn(rowClassName, 'h-auto justify-start rounded-none px-0')}
            aria-label={`Select ${preview.subject} from ${preview.sender} to reply`}
            aria-pressed={active}
            data-hail-mail-list-item="true"
            data-hail-thread-id={item.thread_id}
          >
            {rowContent}
          </Button>
      ) : (
        // Set Aside: clicking navigates to thread
        <Link
          to="/thread/$threadId"
          params={{ threadId: item.thread_id }}
          search={{ from: kind }}
          className={rowClassName}
          aria-label={`Open ${preview.subject} from ${preview.sender}`}
          data-hail-mail-list-item="true"
          data-hail-thread-id={item.thread_id}
        >
          {rowContent}
        </Link>
      )}
      <div className="flex shrink-0 items-center pr-1 sm:pr-0">
        <Button
          type="button"
          variant="outline"
          size="xs"
          className="opacity-90 sm:opacity-0 sm:group-hover:opacity-100 sm:group-focus-within:opacity-100"
          onClick={() => moveBack.mutate({ threadId: item.thread_id, to: 'imbox' })}
          disabled={moveBack.isPending}
        >
          {moveBack.isPending ? 'Moving…' : config.actionLabel}
        </Button>
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
    <div className="flex min-h-[400px] gap-0">
      {/* Left: thread list */}
        <div className={`min-w-0 ${selectedItem ? 'w-2/5 border-r border-border' : 'w-full'}`}>
        <ActionableList
          items={data.items}
          actions={{ client, availableActions: ['archive', 'trash', 'classify'] }}
          renderItem={(item, { selected, onToggleSelect }) => (
            <PileRow
              item={item}
              config={config}
              kind="reply-later"
              selected={selected}
              active={item.thread_id === selectedId}
              client={client}
              onSelect={() => setSelectedId(item.thread_id === selectedId ? null : item.thread_id)}
              onToggleSelect={onToggleSelect}
            />
          )}
          keyExtractor={(item) => item.thread_id}
          emptyState={<StateCard title={config.emptyTitle} body={config.emptyBody} />}
        />
      </div>

      {/* Right: reply panel */}
      {selectedItem ? (
        <div className="w-3/5">
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
  client,
}: {
  query: ReturnType<SectionConfig['useView']>;
  config: SectionConfig;
  kind: PileSectionPageProps['kind'];
  client: HailApiClient;
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
    <ActionableList
      items={data.items}
      actions={{ client, availableActions: ['archive', 'trash', 'classify'] }}
      renderItem={(item, { selected, onToggleSelect }) => (
        <PileRow
          item={item}
          config={config}
          kind={kind}
          selected={selected}
          client={client}
          onToggleSelect={onToggleSelect}
        />
      )}
      keyExtractor={(item) => item.thread_id}
      emptyState={<StateCard title={config.emptyTitle} body={config.emptyBody} />}
    />
  );
}

// ---------------------------------------------------------------------------
// Page
// ---------------------------------------------------------------------------

export function PileSectionPage({ kind }: PileSectionPageProps) {
  const config = configs[kind];
  const client = useApiClient();
  const query = config.useView(client);

  const list =
    kind === 'reply-later' ? (
      <ReplyLaterList query={query} config={config} client={client} />
    ) : (
      <PileList query={query} config={config} kind={kind} client={client} />
    );

  return (
    <AppShell
      title={config.title}
      description={config.description}
      list={list}
      wide={kind === 'reply-later'}
    />
  );
}
