import { useEffect, useRef, useState } from 'react';
import { cn } from '../lib/utils';
import {
  type FeedBlockedTracker,
  type HailApiClient,
  type MailViewItem,
  type MailViewKind,
} from '../api/client';
import {
  useFeedView,
  useImboxSectioned,
  usePapertrailSectioned,
  useScreenerView,
  useUserPrefs,
} from '../api/query';
import { useApiClient } from '../api/ApiClientProvider';
import { ActionableList } from '../components/ActionableList';
import { EmailFrame } from '../components/EmailFrame';
import { ErrorState } from '../components/ErrorState';
import { ArrowUpCircle } from '../components/icons';
import { LoadingState } from '../components/LoadingState';
import { StateCard } from '../components/StateCard';
import { ThreadLink } from '../components/ThreadLink';
import { MailRow as SharedMailRow, MailRowQuickActionsMenu } from '../components/MailRow';
import { PowerThroughFeed } from '../components/PowerThroughFeed';
import { ScreenerBanner } from '../components/ScreenerBanner';
import { AppShell } from '../layout/AppShell';
import { Button } from '../components/ui/button';
import { Badge } from '../components/ui/badge';
import {
  Card,
  CardAction,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from '../components/ui/card';
import { actionErrorMessage, viewErrorMessage } from '../lib/errorMessages';

interface MailViewPageProps {
  view: MailViewKind;
  title: string;
  description: string;
  client?: HailApiClient;
}

const viewLabels: Record<MailViewKind, string> = {
  imbox: 'Imbox',
  feed: 'Feed',
  papertrail: 'Paper Trail',
};

const emptyStates: Record<MailViewKind, { title: string; body: string }> = {
  imbox: {
    title: "You're all caught up.",
    body: 'New mail will appear here.',
  },
  feed: {
    title: 'Nothing in The Feed yet.',
    body: 'Newsletters and notifications will show up here.',
  },
  papertrail: {
    title: 'No receipts yet.',
    body: 'Transactional mail will land here.',
  },
};

function useMailView(view: MailViewKind, client?: HailApiClient) {
  switch (view) {
    case 'imbox':
      return useImboxSectioned(client);
    case 'feed':
      return useFeedView(client);
    case 'papertrail':
      return usePapertrailSectioned(client);
  }
}

function classificationLabel(classification: string) {
  return (viewLabels as Record<string, string>)[classification] ?? classification;
}

const FEED_COLLAPSED_MAX_HEIGHT = 420;

function useFeedSeenObserver({
  items,
  client,
}: {
  items: MailViewItem[];
  client: HailApiClient;
}) {
  const markedRef = useRef<Set<string>>(new Set());
  const elementsRef = useRef<Map<string, HTMLElement>>(new Map());
  const [errors, setErrors] = useState<Record<string, Error>>({});

  useEffect(() => {
    markedRef.current = new Set(
      [...markedRef.current].filter((threadId) =>
        items.some((item) => item.thread_id === threadId),
      ),
    );
  }, [items]);

  useEffect(() => {
    if (typeof IntersectionObserver === 'undefined') {
      return;
    }

    const observer = new IntersectionObserver(
      (entries) => {
        for (const entry of entries) {
          if (entry.isIntersecting || entry.boundingClientRect.bottom > 0) {
            continue;
          }
          const threadId = (entry.target as HTMLElement).dataset.hailFeedThreadId;
          if (!threadId || markedRef.current.has(threadId)) {
            continue;
          }
          markedRef.current.add(threadId);
          void client.markThread(threadId, true).catch((error: Error) => {
            markedRef.current.delete(threadId);
            setErrors((current) => ({ ...current, [threadId]: error }));
          });
        }
      },
      { threshold: 0 },
    );

    for (const element of elementsRef.current.values()) {
      observer.observe(element);
    }

    return () => observer.disconnect();
  }, [client, items]);

  function register(threadId: string) {
    return (element: HTMLElement | null) => {
      if (element) {
        elementsRef.current.set(threadId, element);
      } else {
        elementsRef.current.delete(threadId);
      }
    };
  }

  return { errors, register };
}

function TrackerSummary({ trackers }: { trackers: FeedBlockedTracker[] }) {
  if (trackers.length === 0) {
    return null;
  }

  return (
    <Badge
      variant="secondary"
      title={trackers.map((tracker) => tracker.reason).join('\n')}
    >
      {trackers.length} tracker{trackers.length === 1 ? '' : 's'} blocked
    </Badge>
  );
}

function isLongFeedHtml(html: string) {
  return html.length > 1200;
}

function FeedCard({
  item,
  register,
  markError,
}: {
  item: MailViewItem;
  register: (threadId: string) => (element: HTMLElement | null) => void;
  markError?: Error;
}) {
  const [expanded, setExpanded] = useState(false);
  const [showImages, setShowImages] = useState(false);
  const feedHtml = (showImages ? item.feed_html_with_images : item.feed_html)?.trim() || '';
  const trackers = item.feed_blocked_trackers ?? [];
  const blockedImages = item.feed_blocked_images ?? 0;
  const shouldClamp = isLongFeedHtml(feedHtml);
  const clamped = shouldClamp && !expanded;

  return (
    <article
      ref={register(item.thread_id)}
      data-hail-feed-thread-id={item.thread_id}
    >
      <Card size="sm" className="gap-0 border border-border py-0 shadow-none ring-0">
        <CardHeader className="border-b border-border p-4 pb-3 sm:p-5 sm:pb-3">
          <div className="min-w-0">
            <CardDescription className="truncate font-medium">
              {item.from || 'Unknown sender'}
            </CardDescription>
            <CardTitle className="mt-1 text-lg font-semibold tracking-tight text-foreground">
              <ThreadLink
                threadId={item.thread_id}
                className="focus-ring rounded-sm outline-none hover:text-primary"
                ariaLabel={`Open ${item.subject || 'thread'} from ${item.from || 'unknown sender'}`}
              >
                {item.subject || '(no subject)'}
              </ThreadLink>
            </CardTitle>
          </div>
          <CardAction className="flex flex-col items-end gap-2">
            {item.unread ? <Badge>New</Badge> : null}
            {item.message_count > 1 ? (
              <Badge variant="outline">{item.message_count} messages</Badge>
            ) : null}
            {item.unread_count > 0 ? (
              <Badge variant="secondary">{item.unread_count} unread</Badge>
            ) : null}
            <TrackerSummary trackers={trackers} />
          </CardAction>
          <ScreenReaderThreadMetadata item={item} />
        </CardHeader>

        <CardContent className="p-4 sm:p-5">
          {blockedImages > 0 && !showImages ? (
            <div className="mb-4 flex flex-wrap items-center gap-3 rounded-md border border-dashed border-border bg-muted/40 px-3 py-2 text-sm text-muted-foreground">
              <span>{blockedImages} image{blockedImages === 1 ? '' : 's'} blocked for privacy.</span>
              <Button type="button" variant="outline" size="sm" onClick={() => setShowImages(true)}>
                Show images ({blockedImages} blocked)
              </Button>
            </div>
          ) : null}
          {feedHtml.length > 0 ? (
            <div className="relative">
              <div
                className={cn(
                  'relative overflow-hidden',
                  clamped && 'after:pointer-events-none after:absolute after:inset-x-0 after:bottom-0 after:h-20 after:bg-gradient-to-b after:from-transparent after:to-card',
                )}
                style={clamped ? { maxHeight: FEED_COLLAPSED_MAX_HEIGHT } : undefined}
              >
                <EmailFrame
                  html={feedHtml}
                  title={`Email body from ${item.from || 'Unknown sender'}`}
                />
              </div>
              {clamped ? (
                <div className="mt-4 flex justify-center">
                  <Button type="button" variant="outline" onClick={() => setExpanded(true)}>
                    Show more
                  </Button>
                </div>
              ) : null}
            </div>
          ) : (
            <p className="whitespace-pre-wrap text-base leading-relaxed text-foreground">
              {item.preview || 'This message has no renderable body.'}
            </p>
          )}

          {markError ? (
            <p role="alert" className="mt-4 text-sm text-destructive">
              {actionErrorMessage(markError, 'Mark read')}
            </p>
          ) : null}
        </CardContent>
      </Card>
    </article>
  );
}

function FeedReadingStream({
  items,
  client,
  emptyState,
}: {
  items: MailViewItem[];
  client: HailApiClient;
  emptyState: { title: string; body: string };
}) {
  const { errors, register } = useFeedSeenObserver({ items, client });

  if (items.length === 0) {
    return <StateCard title={emptyState.title} body={emptyState.body} />;
  }

  return (
    <div className="flex flex-col gap-4 sm:gap-5">
      {items.map((item) => (
        <FeedCard
          key={item.thread_id}
          item={item}
          register={register}
          markError={errors[item.thread_id]}
        />
      ))}
    </div>
  );
}

function MailListRow({
  item,
  view,
  selected,
  onToggleSelect,
  client,
}: {
  item: MailViewItem;
  view: MailViewKind;
  selected?: boolean;
  onToggleSelect?: () => void;
  client?: HailApiClient;
}) {
  return (
    <MailThreadRow
      item={item}
      view={view}
      selected={selected}
      onToggleSelect={onToggleSelect}
      client={client}
    />
  );
}

function ScreenReaderThreadMetadata({ item }: { item: MailViewItem }) {
  return (
    <span className="sr-only">
      <span>{classificationLabel(item.classification)}</span>
      <span role="img" aria-label={item.unread ? 'Unread thread' : 'Read thread'} />
      {item.unread ? <span>Unread</span> : null}
      {item.message_count > 1 ? <span>{item.message_count} messages</span> : null}
      {item.unread_count > 0 ? <span>{item.unread_count} unread messages</span> : null}
    </span>
  );
}

function rowDensityClass(view: MailViewKind) {
  if (view === 'feed') {
    return 'py-1.5';
  }

  if (view === 'papertrail') {
    return 'py-0.5';
  }

  return 'py-1';
}

function MailThreadRow({
  item,
  view,
  selected,
  onToggleSelect,
  client,
}: {
  item: MailViewItem;
  view: MailViewKind;
  selected?: boolean;
  onToggleSelect?: () => void;
  client?: HailApiClient;
}) {
  return (
    <div
      className={cn(
        'group/mail-row flex items-stretch border-b border-l-2 border-b-border border-l-transparent hover:bg-muted/60 focus-within:bg-accent',
        rowDensityClass(view),
        selected && 'bg-accent',
      )}
    >
      <ThreadLink
        threadId={item.thread_id}
        mailListItem
        className="block min-w-0 flex-1 focus-visible:border-l-primary focus-visible:outline-none"
        ariaLabel={`Open ${item.subject || 'thread'} from ${item.from || 'unknown sender'}`}
      >
        <ScreenReaderThreadMetadata item={item} />
        <SharedMailRow
          from={item.from || 'Unknown sender'}
          subject={item.subject || '(no subject)'}
          preview={view === 'papertrail' ? '' : item.preview || 'No preview available.'}
          receivedAt={item.received_at}
          unread={view === 'papertrail' ? false : item.unread}
          hasNotes={item.has_notes}
          selected={selected}
          onToggleSelect={onToggleSelect}
          labels={item.labels}
          messageCount={item.message_count}
          unreadCount={item.unread_count}
        />
      </ThreadLink>
      <div className="flex shrink-0 items-start px-2 py-2">
        <MailRowQuickActionsMenu
          threadId={item.thread_id}
          subject={item.subject || '(no subject)'}
          unread={view === 'papertrail' ? false : item.unread}
          selected={Boolean(selected)}
          client={client}
        />
      </div>
    </div>
  );
}


function isSectionedImboxData(data: unknown): data is {
  bubbled_up: MailViewItem[];
  new_for_you: MailViewItem[];
  previously_seen: MailViewItem[];
  new_count: number;
  previously_seen_total: number;
} {
  return Boolean(
    data &&
      typeof data === 'object' &&
      'bubbled_up' in data &&
      'new_for_you' in data &&
      'previously_seen' in data,
  );
}

function isSectionedMailViewData(data: unknown): data is {
  bubble_up?: MailViewItem[];
  new: MailViewItem[];
  seen: MailViewItem[];
  next_cursor: string | null;
} {
  return Boolean(
    data &&
      typeof data === 'object' &&
      'new' in data &&
      'seen' in data,
  );
}

function isFlatMailViewData(data: unknown): data is { items: MailViewItem[] } {
  return Boolean(
    data &&
      typeof data === 'object' &&
      'items' in data,
  );
}

function getFlatViewItems(data: ReturnType<typeof useMailView>['data']): MailViewItem[] {
  return isFlatMailViewData(data) ? data.items : [];
}

function MailRows({
  items,
  view,
  client,
}: {
  items: MailViewItem[];
  view: MailViewKind;
  client?: HailApiClient;
}) {
  return (
    <ActionableList
      items={items}
      actions={{
        client,
        availableActions: ['archive', 'trash', 'set-aside', 'reply-later', 'classify'],
      }}
      renderItem={(item, { selected, onToggleSelect }) => (
        <MailListRow
          item={item}
          view={view}
          selected={selected}
          onToggleSelect={onToggleSelect}
          client={client}
        />
      )}
      keyExtractor={(item) => item.thread_id}
      emptyState={null}
    />
  );
}

export function SectionedListView({
  sections,
  view,
  client,
  emptyState,
  onLoadMoreSeen,
}: {
  sections: {
    bubbleUp?: MailViewItem[];
    new: MailViewItem[];
    seen: MailViewItem[];
  };
  view: MailViewKind;
  client?: HailApiClient;
  emptyState: { title: string; body: string };
  onLoadMoreSeen?: () => void;
}) {
  const hasAnyItems =
    (sections.bubbleUp?.length ?? 0) + sections.new.length + sections.seen.length > 0;

  if (!hasAnyItems) {
    return <StateCard title={emptyState.title} body={emptyState.body} />;
  }

  return (
    <div className="flex flex-col gap-5">
      {sections.bubbleUp && sections.bubbleUp.length > 0 ? (
        <section aria-labelledby={`${view}-bubble-up-heading`}>
          <div className="mb-2 flex items-center gap-2 px-1">
            <ArrowUpCircle className="text-primary" aria-hidden="true" />
            <h2
              id={`${view}-bubble-up-heading`}
              className="text-xs font-medium uppercase tracking-wide text-muted-foreground"
            >
              Bubbled Up
            </h2>
          </div>
          <MailRows items={sections.bubbleUp} view={view} client={client} />
        </section>
      ) : null}

      {sections.new.length > 0 ? (
        <section aria-label="New">
          <MailRows items={sections.new} view={view} client={client} />
        </section>
      ) : null}

      {sections.seen.length > 0 ? (
        <section aria-labelledby={`${view}-previously-read-heading`}>
          <div className="mb-2 px-1">
            <h2
              id={`${view}-previously-read-heading`}
              className="text-xs font-medium uppercase tracking-wide text-muted-foreground"
            >
              Previously read
            </h2>
          </div>
          <MailRows items={sections.seen} view={view} client={client} />
          {onLoadMoreSeen ? (
            <div className="mt-3 flex justify-center">
              <Button type="button" variant="ghost" size="sm" onClick={onLoadMoreSeen}>
                Load more read mail
              </Button>
            </div>
          ) : null}
        </section>
      ) : null}
    </div>
  );
}

function ImboxSectionedList({
  data,
  client,
  powerThrough,
  onStartPowerThrough,
  onExitPowerThrough,
}: {
  data: {
    bubbled_up: MailViewItem[];
    new_for_you: MailViewItem[];
    previously_seen: MailViewItem[];
    new_count: number;
    previously_seen_total: number;
  };
  client: HailApiClient;
  powerThrough: boolean;
  onStartPowerThrough: () => void;
  onExitPowerThrough: () => void;
}) {
  const bubbledUp = data.bubbled_up;
  const newForYou = data.new_for_you;
  const previouslySeen = data.previously_seen;
  const newCount = data.new_count;
  const previouslySeenTotal = data.previously_seen_total;
  const hiddenPreviouslySeen = Math.max(previouslySeenTotal - previouslySeen.length, 0);

  return (
    <div className="flex flex-col gap-5">
      {bubbledUp.length > 0 ? (
        <section aria-labelledby="imbox-bubbled-up-heading">
          <div className="mb-2 flex items-center gap-2 px-1">
            <ArrowUpCircle className="text-primary" aria-hidden="true" />
            <h2
              id="imbox-bubbled-up-heading"
              className="text-xs font-medium uppercase tracking-wide text-muted-foreground"
            >
              Bubbled Up
            </h2>
          </div>
          <MailRows
            items={bubbledUp}
            view="imbox"
            client={client}
          />
        </section>
      ) : null}

      <section aria-labelledby="imbox-new-for-you-heading">
        <div className="mb-2 flex items-center justify-between gap-3 px-1">
          <div className="flex items-center gap-2">
            <h2
              id="imbox-new-for-you-heading"
              className="text-xs font-medium uppercase tracking-wide text-muted-foreground"
            >
              New for you
            </h2>
            {newCount > 0 ? (
              <Badge>{newCount}</Badge>
            ) : null}
          </div>
          {newForYou.length > 0 && (
            <Button type="button" onClick={onStartPowerThrough} size="sm">
              Power through new ({newForYou.length})
            </Button>
          )}
        </div>
        {powerThrough ? (
          <PowerThroughFeed
            items={newForYou}
            client={client}
            onExit={onExitPowerThrough}
          />
        ) : newForYou.length === 0 ? (
          <StateCard title="You're all caught up." body="New mail will appear here." />
        ) : (
          <MailRows
            items={newForYou}
            view="imbox"
            client={client}
          />
        )}
      </section>

      {previouslySeen.length > 0 ? (
        <section aria-labelledby="imbox-previously-seen-heading">
          <div className="mb-2 flex items-center justify-between px-1">
            <h2
              id="imbox-previously-seen-heading"
              className="text-xs font-medium uppercase tracking-wide text-muted-foreground"
            >
              Previously seen
            </h2>
            {hiddenPreviouslySeen > 0 ? (
              <span className="text-xs text-muted-foreground">{hiddenPreviouslySeen} more</span>
            ) : null}
          </div>
          <MailRows
            items={previouslySeen}
            view="imbox"
            client={client}
          />
        </section>
      ) : null}
    </div>
  );
}

export function MailViewPage({
  view,
  title,
  description,
  client,
}: MailViewPageProps) {
  const contextClient = useApiClient();
  const apiClient = client ?? contextClient;
  const query = useMailView(view, apiClient);
  useUserPrefs(apiClient, { enabled: view === 'feed' });
  const screenerQuery = useScreenerView(apiClient);
  const pendingCount = screenerQuery.data?.senders?.length ?? 0;
  const [powerThrough, setPowerThrough] = useState(false);

  useEffect(() => {
    if (view !== 'imbox') {
      setPowerThrough(false);
    }
  }, [view]);

  function startPowerThrough() {
    setPowerThrough(true);
  }

  function exitPowerThrough() {
    setPowerThrough(false);
  }

  let list;
  if (query.isPending) {
    list = <LoadingState label={`Loading ${viewLabels[view]} mail`} />;
  } else if (query.isError) {
    list = (
      <ErrorState
        message={viewErrorMessage(query.error, 'Mail view')}
        onRetry={() => void query.refetch()}
      />
    );
  } else {
    const emptyState = emptyStates[view];
    list = (
      <div>
        {view === 'imbox' ? <ScreenerBanner pendingCount={pendingCount} /> : null}
        {view === 'imbox' && isSectionedImboxData(query.data) ? (
          <ImboxSectionedList
            data={query.data}
            client={apiClient}
            powerThrough={powerThrough}
            onStartPowerThrough={startPowerThrough}
            onExitPowerThrough={exitPowerThrough}
          />
        ) : view === 'feed' ? (
          <FeedReadingStream
            items={getFlatViewItems(query.data)}
            client={apiClient}
            emptyState={emptyState}
          />
        ) : isSectionedMailViewData(query.data) ? (
          <SectionedListView
            sections={{
              bubbleUp: query.data.bubble_up,
              new: query.data.new,
              seen: query.data.seen,
            }}
            view={view}
            client={apiClient}
            emptyState={emptyState}
          />
        ) : (
          <ActionableList
            items={getFlatViewItems(query.data)}
            actions={{
              client: apiClient,
              availableActions: ['archive', 'trash', 'set-aside', 'reply-later', 'classify'],
            }}
            renderItem={(item, { selected, onToggleSelect }) => (
              <MailListRow
                item={item}
                view={view}
                selected={selected}
                onToggleSelect={onToggleSelect}
                client={apiClient}
              />
            )}
            keyExtractor={(item) => item.thread_id}
            emptyState={<StateCard title={emptyState.title} body={emptyState.body} />}
          />
        )}
      </div>
    );
  }

  return (
    <AppShell
      title={title}
      description={description}
      list={list}
    />
  );
}
