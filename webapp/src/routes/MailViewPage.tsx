import { useEffect, useState } from 'react';
import { cn } from '../lib/utils';
import {
  type HailApiClient,
  type MailViewItem,
  type MailViewKind,
  type ThreadVerbResponse,
} from '../api/client';
import {
  useClassifyThreadMutation,
  useFeedView,
  useImboxSectioned,
  usePapertrailView,
  useScreenerView,
  useSetAsideThreadMutation,
  useTrashThreadMutation,
} from '../api/query';
import { useApiClient } from '../api/ApiClientProvider';
import { ActionableList } from '../components/ActionableList';
import { ErrorState } from '../components/ErrorState';
import { ArrowUpCircle } from '../components/icons';
import { LoadingState } from '../components/LoadingState';
import { StateCard } from '../components/StateCard';
import { ThreadLink } from '../components/ThreadLink';
import { MailRow as SharedMailRow } from '../components/MailRow';
import { ScreenerBanner } from '../components/ScreenerBanner';
import { useUndoToast } from '../components/UndoToastProvider';
import { AppShell } from '../layout/AppShell';
import { Button } from '../components/ui/button';
import { Badge } from '../components/ui/badge';
import { Alert, AlertDescription, AlertTitle } from '../components/ui/alert';
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
      return usePapertrailView(client);
  }
}

function classificationLabel(classification: string) {
  return (viewLabels as Record<string, string>)[classification] ?? classification;
}

function MailListRow({
  item,
  view,
  selected,
  onToggleSelect,
}: {
  item: MailViewItem;
  view: MailViewKind;
  selected?: boolean;
  onToggleSelect?: () => void;
}) {
  return (
    <MailThreadRow
      item={item}
      view={view}
      selected={selected}
      onToggleSelect={onToggleSelect}
    />
  );
}

function ScreenReaderThreadMetadata({ item }: { item: MailViewItem }) {
  return (
    <span className="sr-only">
      <span>{classificationLabel(item.classification)}</span>
      <span role="img" aria-label={item.unread ? 'Unread thread' : 'Read thread'} />
      {item.unread ? <span>Unread</span> : null}
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
}: {
  item: MailViewItem;
  view: MailViewKind;
  selected?: boolean;
  onToggleSelect?: () => void;
}) {
  return (
    <ThreadLink
      threadId={item.thread_id}
      mailListItem
      className={cn(
        'block border-b border-l-2 border-b-border border-l-transparent focus-visible:border-l-primary focus-visible:bg-accent focus-visible:outline-none hover:bg-muted/60',
        rowDensityClass(view),
        selected && 'bg-accent',
      )}
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
      />
    </ThreadLink>
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

function getFlatViewItems(data: ReturnType<typeof useMailView>['data']): MailViewItem[] {
  if (!data || isSectionedImboxData(data)) {
    return [];
  }

  return data.items;
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
        />
      )}
      keyExtractor={(item) => item.thread_id}
      emptyState={null}
    />
  );
}

type PowerThroughAction = 'imbox' | 'feed' | 'papertrail' | 'set-aside' | 'trash';

interface PowerThroughStatus {
  busy: boolean;
  error: Error | null;
}

function PowerThroughCard({
  item,
  index,
  total,
  busy = false,
  error = null,
  onAction,
  onExit,
}: {
  item: MailViewItem;
  index: number;
  total: number;
  busy?: boolean;
  error?: Error | null;
  onAction: (action: PowerThroughAction) => void;
  onExit: () => void;
}) {
  return (
    <Alert className="shadow-sm">
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0">
          <AlertTitle>{index + 1} of {total}</AlertTitle>
          <AlertDescription className="mt-2">
            <span className="block text-sm font-medium text-foreground">
              {item.from || 'Unknown sender'}
            </span>
            <span className="mt-1 block text-sm text-foreground">
              {item.subject || '(no subject)'}
            </span>
            <span className="mt-1 line-clamp-2 block text-xs text-muted-foreground">
              {item.preview || 'No preview available.'}
            </span>
          </AlertDescription>
        </div>
        <Button type="button" variant="ghost" size="sm" onClick={onExit}>
          Exit
        </Button>
      </div>
      <div className="mt-4 flex flex-wrap gap-2">
        <Button type="button" disabled={busy} onClick={() => onAction('imbox')} size="sm">
          Keep in Imbox
        </Button>
        <Button type="button" disabled={busy} onClick={() => onAction('feed')} variant="outline" size="sm">
          Move to Feed
        </Button>
        <Button type="button" disabled={busy} onClick={() => onAction('papertrail')} variant="outline" size="sm">
          Move to Paper Trail
        </Button>
        <Button type="button" disabled={busy} onClick={() => onAction('set-aside')} variant="outline" size="sm">
          Set Aside
        </Button>
        <Button type="button" disabled={busy} onClick={() => onAction('trash')} variant="destructive" size="sm">
          Trash
        </Button>
      </div>
      {error ? (
        <p role="alert" className="mt-3 text-sm text-destructive">
          {actionErrorMessage(error, 'Thread action')}
        </p>
      ) : null}
    </Alert>
  );
}

function ImboxSectionedList({
  data,
  client,
  powerThrough,
  ptIndex,
  powerThroughStatus,
  onStartPowerThrough,
  onPowerThroughAction,
  onExitPowerThrough,
}: {
  data: {
    bubbled_up: MailViewItem[];
    new_for_you: MailViewItem[];
    previously_seen: MailViewItem[];
    new_count: number;
    previously_seen_total: number;
  };
  client?: HailApiClient;
  powerThrough: boolean;
  ptIndex: number;
  powerThroughStatus: PowerThroughStatus;
  onStartPowerThrough: () => void;
  onPowerThroughAction: (action: PowerThroughAction) => void;
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
          ptIndex < newForYou.length ? (
            <PowerThroughCard
              item={newForYou[ptIndex]}
              index={ptIndex}
              total={newForYou.length}
              busy={powerThroughStatus.busy}
              error={powerThroughStatus.error}
              onAction={onPowerThroughAction}
              onExit={onExitPowerThrough}
            />
          ) : (
            <StateCard title="All done!" body="You powered through every new thread." />
          )
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
  const undoToast = useUndoToast();
  const screenerQuery = useScreenerView(apiClient);
  const pendingCount = screenerQuery.data?.senders?.length ?? 0;
  const [powerThrough, setPowerThrough] = useState(false);
  const [ptIndex, setPtIndex] = useState(0);
  const [showPowerThroughDone, setShowPowerThroughDone] = useState(false);

  useEffect(() => {
    if (view !== 'imbox') {
      setPowerThrough(false);
      setPtIndex(0);
      setShowPowerThroughDone(false);
    }
  }, [view]);


  const classifyPowerThrough = useClassifyThreadMutation(apiClient);
  const setAsidePowerThrough = useSetAsideThreadMutation(apiClient);
  const trashPowerThrough = useTrashThreadMutation(apiClient);
  const [markSeenPowerThrough, setMarkSeenPowerThrough] = useState<PowerThroughStatus>({
    busy: false,
    error: null,
  });
  const powerThroughStatus = {
    busy:
      markSeenPowerThrough.busy ||
      classifyPowerThrough.isPending ||
      setAsidePowerThrough.isPending ||
      trashPowerThrough.isPending,
    error:
      markSeenPowerThrough.error ??
      classifyPowerThrough.error ??
      setAsidePowerThrough.error ??
      trashPowerThrough.error,
  } satisfies PowerThroughStatus;

  function startPowerThrough() {
    setPowerThrough(true);
    setPtIndex(0);
    setShowPowerThroughDone(false);
    setMarkSeenPowerThrough({ busy: false, error: null });
  }

  function exitPowerThrough() {
    setPowerThrough(false);
    setPtIndex(0);
    setShowPowerThroughDone(false);
    setMarkSeenPowerThrough({ busy: false, error: null });
  }

  function advancePowerThrough() {
    setPtIndex((index) => index + 1);
  }

  function completePowerThrough() {
    setPowerThrough(false);
    setPtIndex(0);
    setMarkSeenPowerThrough({ busy: false, error: null });
    setShowPowerThroughDone(true);
    window.setTimeout(() => setShowPowerThroughDone(false), 1500);
  }

  function handlePowerThroughSuccess(
    message: string,
    data?: ThreadVerbResponse,
    undoSuccessMessage?: string,
  ) {
    undoToast.showToast({
      message,
      undo: data?.undo ? { id: data.undo.id } : null,
      undoSuccessMessage,
    });
    if (isSectionedImboxData(query.data) && ptIndex + 1 >= query.data.new_for_you.length) {
      completePowerThrough();
    } else {
      advancePowerThrough();
    }
  }

  function handlePowerThroughAction(action: PowerThroughAction) {
    if (!isSectionedImboxData(query.data) || powerThroughStatus.busy) {
      return;
    }

    const item = query.data.new_for_you[ptIndex];
    if (!item) {
      completePowerThrough();
      return;
    }

    if (action === 'imbox') {
      setMarkSeenPowerThrough({ busy: true, error: null });
      void apiClient.markThread(item.thread_id, true)
        .then(() => handlePowerThroughSuccess('Kept thread in Imbox.'))
        .catch((error: Error) => setMarkSeenPowerThrough({ busy: false, error }))
        .finally(() => {
          setMarkSeenPowerThrough((status) => ({ ...status, busy: false }));
        });
      return;
    }

    if (action === 'feed' || action === 'papertrail') {
      classifyPowerThrough.mutate(
        { threadId: item.thread_id, to: action },
        {
          onSuccess: (data, variables) =>
            handlePowerThroughSuccess(
              `Moved thread to ${viewLabels[variables.to]}.`,
              data,
              'Thread classification undone.',
            ),
        },
      );
      return;
    }

    if (action === 'set-aside') {
      setAsidePowerThrough.mutate(
        { threadId: item.thread_id },
        {
          onSuccess: (data) =>
            handlePowerThroughSuccess(
              'Thread added to Set Aside.',
              data,
              'Set Aside undone.',
            ),
        },
      );
      return;
    }

    trashPowerThrough.mutate(
      { threadId: item.thread_id },
      {
        onSuccess: (data) =>
          handlePowerThroughSuccess(
            'Thread moved to trash.',
            data,
            'Trash undone.',
          ),
      },
    );
  }

  useEffect(() => {
    if (!powerThrough) {
      return;
    }

    function handlePowerThroughKeydown(event: KeyboardEvent) {
      if (event.defaultPrevented) {
        return;
      }

      const target = event.target;
      if (
        target instanceof HTMLInputElement ||
        target instanceof HTMLTextAreaElement ||
        target instanceof HTMLSelectElement ||
        (target instanceof HTMLElement && target.isContentEditable)
      ) {
        return;
      }

      const shortcuts: Record<string, PowerThroughAction> = {
        '1': 'imbox',
        '2': 'feed',
        '3': 'papertrail',
        '4': 'set-aside',
        '5': 'trash',
      };

      if (event.key === 'Escape') {
        event.preventDefault();
        exitPowerThrough();
        return;
      }

      const action = shortcuts[event.key];
      if (action) {
        event.preventDefault();
        handlePowerThroughAction(action);
      }
    }

    window.addEventListener('keydown', handlePowerThroughKeydown);
    return () => window.removeEventListener('keydown', handlePowerThroughKeydown);
  }, [powerThrough, powerThroughStatus.busy, ptIndex, query.data]);

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
        {view === 'imbox' && showPowerThroughDone ? (
          <div className="mb-4">
            <StateCard title="All done!" body="You powered through every new thread." />
          </div>
        ) : null}
        {view === 'imbox' ? <ScreenerBanner pendingCount={pendingCount} /> : null}
        {view === 'imbox' && isSectionedImboxData(query.data) ? (
          <ImboxSectionedList
            data={query.data}
            client={apiClient}
            powerThrough={powerThrough}
            ptIndex={ptIndex}
            powerThroughStatus={powerThroughStatus}
            onStartPowerThrough={startPowerThrough}
            onPowerThroughAction={handlePowerThroughAction}
            onExitPowerThrough={exitPowerThrough}
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
