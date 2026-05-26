import { useQueryClient } from '@tanstack/react-query';
import { useEffect, useMemo, useState } from 'react';
import {
  type HailApiClient,

  type MailViewItem,
  type MailViewKind,
  type ThreadVerbResponse,
} from '../api/client';
import {
  useArchiveThreadMutation,
  useClassifyThreadMutation,
  useFeedView,
  useImboxSectioned,
  usePapertrailView,
  useReplyLaterThreadMutation,
  useScreenerView,
  useSetAsideThreadMutation,
  useTrashThreadMutation,
  defaultApiClient,
} from '../api/query';
import { queryKeys } from '../api/queryKeys';
import { BatchActionBar } from '../components/BatchActionBar';
import { ErrorState } from '../components/ErrorState';
import { ArrowUpCircle } from '../components/icons';
import { LoadingState } from '../components/LoadingState';
import { StateCard } from '../components/StateCard';
import { ThreadLink } from '../components/ThreadLink';
import { ListView } from '../components/ListView';
import { MailRow as SharedMailRow } from '../components/MailRow';
import { ScreenerBanner } from '../components/ScreenerBanner';
import { useUndoToast } from '../components/UndoToastProvider';
import { AppShell } from '../layout/AppShell';
import { pillButtonClass } from '../lib/buttonStyles';
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

function ShortcutActionButton({
  action,
  label,
  busy,
  onClick,
}: {
  action: string;
  label: string;
  busy: boolean;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      data-hail-shortcut-action={action}
      disabled={busy}
      onClick={(event) => {
        event.preventDefault();
        event.stopPropagation();
        onClick();
      }}
      className="sr-only"
      tabIndex={-1}
    >
      {label}
    </button>
  );
}

function ThreadShortcutActions({
  item,
  client,
  onHandled,
}: {
  item: MailViewItem;
  client?: HailApiClient;
  onHandled?: () => void;
}) {
  const undoToast = useUndoToast();
  const archive = useArchiveThreadMutation(client, {
    onSuccess: (data) => {
      undoToast.showToast({
        message: 'Thread archived.',
        undo: data.undo ? { id: data.undo.id } : null,
        undoSuccessMessage: 'Archive undone.',
      });
      onHandled?.();
    },
  });
  const trash = useTrashThreadMutation(client, {
    onSuccess: (data) => {
      undoToast.showToast({
        message: 'Thread moved to trash.',
        undo: data.undo ? { id: data.undo.id } : null,
        undoSuccessMessage: 'Trash undone.',
      });
      onHandled?.();
    },
  });
  const setAside = useSetAsideThreadMutation(client, {
    onSuccess: (data) => {
      undoToast.showToast({
        message: 'Thread added to Set Aside.',
        undo: data.undo ? { id: data.undo.id } : null,
        undoSuccessMessage: 'Set Aside undone.',
      });
      onHandled?.();
    },
  });
  const replyLater = useReplyLaterThreadMutation(client, {
    onSuccess: (data) => {
      undoToast.showToast({
        message: 'Thread added to Reply Later.',
        undo: data.undo ? { id: data.undo.id } : null,
        undoSuccessMessage: 'Reply Later undone.',
      });
      onHandled?.();
    },
  });
  const busy =
    archive.isPending || trash.isPending || setAside.isPending || replyLater.isPending;
  const error = archive.error ?? trash.error ?? setAside.error ?? replyLater.error;

  return (
    <>
      <ShortcutActionButton
        action="archive"
        label="Archive"
        busy={busy}
        onClick={() => archive.mutate({ threadId: item.thread_id })}
      />
      <ShortcutActionButton
        action="trash"
        label="Trash"
        busy={busy}
        onClick={() => trash.mutate({ threadId: item.thread_id })}
      />
      <ShortcutActionButton
        action="set-aside"
        label="Set Aside"
        busy={busy}
        onClick={() => setAside.mutate({ threadId: item.thread_id })}
      />
      <ShortcutActionButton
        action="reply-later"
        label="Reply Later"
        busy={busy}
        onClick={() => replyLater.mutate({ threadId: item.thread_id })}
      />
      {error ? (
        <span role="alert" className="sr-only">
          {actionErrorMessage(error, 'Thread action')}
        </span>
      ) : null}
    </>
  );
}

function MailListRow({
  item,
  view,
  client,
  selected,
  onToggleSelect,
}: {
  item: MailViewItem;
  view: MailViewKind;
  client?: HailApiClient;
  selected?: boolean;
  onToggleSelect?: () => void;
}) {
  return (
    <div className="relative">
      <MailThreadRow
        item={item}
        view={view}
        selected={selected}
        onToggleSelect={onToggleSelect}
      />
      <ThreadShortcutActions item={item} client={client} />
    </div>
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
  if (view === 'feed') {
    return <FeedThreadRow item={item} selected={selected} onToggleSelect={onToggleSelect} />;
  }

  if (view === 'papertrail') {
    return <PaperTrailThreadRow item={item} selected={selected} onToggleSelect={onToggleSelect} />;
  }

  return <ImboxThreadRow item={item} selected={selected} onToggleSelect={onToggleSelect} />;
}

function ImboxThreadRow({
  item,
  selected = false,
  onToggleSelect,
}: {
  item: MailViewItem;
  selected?: boolean;
  onToggleSelect?: () => void;
}) {
  return (
    <ThreadLink
      threadId={item.thread_id}
      mailListItem
      className={`block border-b border-l-[3px] border-b-border-hairline border-l-transparent py-4 pl-3 pr-0 focus-visible:border-l-accent-blue focus-visible:bg-bg-selected focus-visible:outline-none sm:py-5 ${
        selected ? 'bg-accent-blue/10' : 'hover:bg-bg-hover'
      }`}
      ariaLabel={`Open ${item.subject || 'thread'} from ${item.from || 'unknown sender'}`}
    >
      <ScreenReaderThreadMetadata item={item} />
      <SharedMailRow
        from={item.from || 'Unknown sender'}
        subject={item.subject || '(no subject)'}
        preview={item.preview || 'No preview available.'}
        receivedAt={item.received_at}
        unread={item.unread}
        hasNotes={item.has_notes}
        selected={selected}
        onToggleSelect={onToggleSelect}
      />
    </ThreadLink>
  );
}

function FeedThreadRow({
  item,
  selected = false,
  onToggleSelect,
}: {
  item: MailViewItem;
  selected?: boolean;
  onToggleSelect?: () => void;
}) {
  return (
    <ThreadLink
      threadId={item.thread_id}
      mailListItem
      className={`block border-b border-l-[3px] border-b-border-hairline border-l-transparent py-6 pl-3 pr-0 focus-visible:border-l-accent-blue focus-visible:bg-bg-selected focus-visible:outline-none sm:py-7 ${
        selected ? 'bg-accent-blue/10' : 'hover:bg-bg-hover'
      }`}
      ariaLabel={`Open ${item.subject || 'thread'} from ${item.from || 'unknown sender'}`}
    >
      <ScreenReaderThreadMetadata item={item} />
      <SharedMailRow
        from={item.from || 'Unknown sender'}
        subject={item.subject || '(no subject)'}
        preview={item.preview || 'No preview available.'}
        receivedAt={item.received_at}
        unread={item.unread}
        hasNotes={item.has_notes}
        selected={selected}
        onToggleSelect={onToggleSelect}
      />
    </ThreadLink>
  );
}

function PaperTrailThreadRow({
  item,
  selected = false,
  onToggleSelect,
}: {
  item: MailViewItem;
  selected?: boolean;
  onToggleSelect?: () => void;
}) {
  return (
    <ThreadLink
      threadId={item.thread_id}
      mailListItem
      className={`block border-b border-l-[3px] border-b-border-hairline border-l-transparent py-2.5 pl-3 pr-0 focus-visible:border-l-accent-blue focus-visible:bg-bg-selected focus-visible:outline-none sm:py-3 ${
        selected ? 'bg-accent-blue/10' : 'hover:bg-bg-hover'
      }`}
      ariaLabel={`Open ${item.subject || 'thread'} from ${item.from || 'unknown sender'}`}
    >
      <ScreenReaderThreadMetadata item={item} />
      <SharedMailRow
        from={item.from || 'Unknown sender'}
        subject={item.subject || '(no subject)'}
        preview=""
        receivedAt={item.received_at}
        hasNotes={item.has_notes}
        selected={selected}
        onToggleSelect={onToggleSelect}
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

function flattenImboxSections(data: ReturnType<typeof useMailView>['data']): MailViewItem[] {
  if (!isSectionedImboxData(data)) {
    return [];
  }

  return [...data.bubbled_up, ...data.new_for_you, ...data.previously_seen];
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
  selected,
  onToggleSelect,
}: {
  items: MailViewItem[];
  view: MailViewKind;
  client?: HailApiClient;
  selected: Set<string>;
  onToggleSelect: (threadId: string) => void;
}) {
  return (
    <>
      {items.map((item) => (
        <MailListRow
          key={`${item.thread_id}:${item.email_id}`}
          item={item}
          view={view}
          client={client}
          selected={selected.has(item.thread_id)}
          onToggleSelect={() => onToggleSelect(item.thread_id)}
        />
      ))}
    </>
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
    <div className="rounded-xl bg-bg-surface p-6 shadow-lg shadow-ink-primary/10">
      <div className="mb-4 flex items-center justify-between">
        <span className="text-sm font-medium text-ink-secondary">
          {index + 1} of {total}
        </span>
        <button
          type="button"
          onClick={onExit}
          className="text-sm text-ink-tertiary focus-ring outline-none hover:text-ink-primary"
        >
          Exit
        </button>
      </div>
      <div className="mb-6">
        <p className="text-lg font-semibold text-ink-primary">
          {item.from || 'Unknown sender'}
        </p>
        <p className="mt-1 text-base text-ink-primary">{item.subject || '(no subject)'}</p>
        <p className="mt-2 line-clamp-3 text-sm text-ink-secondary">
          {item.preview || 'No preview available.'}
        </p>
      </div>
      <div className="flex flex-wrap gap-2">
        <button
          type="button"
          disabled={busy}
          onClick={() => onAction('imbox')}
          className={pillButtonClass('primary', 'md')}
        >
          Keep in Imbox
        </button>
        <button
          type="button"
          disabled={busy}
          onClick={() => onAction('feed')}
          className={pillButtonClass('outline', 'md')}
        >
          Move to Feed
        </button>
        <button
          type="button"
          disabled={busy}
          onClick={() => onAction('papertrail')}
          className={pillButtonClass('outline', 'md')}
        >
          Move to Paper Trail
        </button>
        <button
          type="button"
          disabled={busy}
          onClick={() => onAction('set-aside')}
          className={pillButtonClass('outline', 'md')}
        >
          Set Aside
        </button>
        <button
          type="button"
          disabled={busy}
          onClick={() => onAction('trash')}
          className={pillButtonClass('danger', 'md')}
        >
          Trash
        </button>
      </div>
      {error ? (
        <p role="alert" className="mt-4 text-sm text-accent-red">
          {actionErrorMessage(error, 'Thread action')}
        </p>
      ) : null}
    </div>
  );
}

function ImboxSectionedList({
  data,
  client,
  selected,
  powerThrough,
  ptIndex,
  powerThroughStatus,
  onToggleSelect,
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
  selected: Set<string>;
  powerThrough: boolean;
  ptIndex: number;
  powerThroughStatus: PowerThroughStatus;
  onToggleSelect: (threadId: string) => void;
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
    <div className="space-y-6">
      {bubbledUp.length > 0 ? (
        <section aria-labelledby="imbox-bubbled-up-heading">
          <div className="mb-3 flex items-center gap-2 px-1">
            <ArrowUpCircle size={16} className="text-accent-yellow" aria-hidden="true" />
            <h2
              id="imbox-bubbled-up-heading"
              className="text-sm font-semibold uppercase tracking-wider text-ink-secondary"
            >
              Bubbled Up
            </h2>
          </div>
          <MailRows
            items={bubbledUp}
            view="imbox"
            client={client}
            selected={selected}
            onToggleSelect={onToggleSelect}
          />
        </section>
      ) : null}

      <section aria-labelledby="imbox-new-for-you-heading">
        <div className="mb-3 flex items-center justify-between px-1">
          <div className="flex items-center gap-2">
            <h2
              id="imbox-new-for-you-heading"
              className="text-sm font-semibold uppercase tracking-wider text-ink-secondary"
            >
              New for you
            </h2>
            {newCount > 0 ? (
              <span className="rounded-full bg-accent-blue px-2 py-0.5 text-xs font-bold text-white">
                {newCount}
              </span>
            ) : null}
          </div>
          {newForYou.length > 0 && (
            <button
              type="button"
              onClick={onStartPowerThrough}
              className={pillButtonClass('primary', 'sm')}
            >
              Power through new ({newForYou.length})
            </button>
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
            selected={selected}
            onToggleSelect={onToggleSelect}
          />
        )}
      </section>

      {previouslySeen.length > 0 ? (
        <section aria-labelledby="imbox-previously-seen-heading">
          <div className="mb-3 flex items-center justify-between px-1">
            <h2
              id="imbox-previously-seen-heading"
              className="text-sm font-semibold uppercase tracking-wider text-ink-tertiary"
            >
              Previously seen
            </h2>
            {hiddenPreviouslySeen > 0 ? (
              <span className="text-xs text-ink-tertiary">{hiddenPreviouslySeen} more</span>
            ) : null}
          </div>
          <MailRows
            items={previouslySeen}
            view="imbox"
            client={client}
            selected={selected}
            onToggleSelect={onToggleSelect}
          />
        </section>
      ) : null}
    </div>
  );
}

function findFocusedThreadId() {
  if (!(document.activeElement instanceof HTMLElement)) {
    return null;
  }

  return document.activeElement.dataset.hailThreadId ?? null;
}

export function MailViewPage({
  view,
  title,
  description,
  client,
}: MailViewPageProps) {
  const query = useMailView(view, client);
  const queryClient = useQueryClient();
  const apiClient = client ?? defaultApiClient;
  const undoToast = useUndoToast();
  const screenerQuery = useScreenerView(client);
  const pendingCount = screenerQuery.data?.senders?.length ?? 0;
  const [powerThrough, setPowerThrough] = useState(false);
  const [ptIndex, setPtIndex] = useState(0);
  const [showPowerThroughDone, setShowPowerThroughDone] = useState(false);
  const [selected, setSelected] = useState<Set<string>>(new Set());

  function toggleSelect(threadId: string) {
    setSelected(prev => {
      const next = new Set(prev);
      if (next.has(threadId)) next.delete(threadId);
      else next.add(threadId);
      return next;
    });
  }

  async function batchAction(action: (threadId: string) => Promise<unknown>) {
    await Promise.all([...selected].map(action));
    setSelected(new Set());
    void queryClient.invalidateQueries({ queryKey: queryKeys.views() });
  }

  function batchActionWithToast(
    action: (threadId: string) => Promise<ThreadVerbResponse>,
    message: string,
  ) {
    void batchAction(async (threadId) => {
      const data = await action(threadId);
      return data;
    }).then(() => {
      undoToast.showToast({ message, undo: null });
    });
  }

  const items = useMemo(() => {
    if (!query.isSuccess) {
      return [];
    }

    return view === 'imbox'
      ? flattenImboxSections(query.data)
      : getFlatViewItems(query.data);
  }, [query.data, query.isSuccess, view]);

  useEffect(() => {
    if (view !== 'imbox') {
      setPowerThrough(false);
      setPtIndex(0);
      setShowPowerThroughDone(false);
    }
    setSelected(new Set());
  }, [view]);

  useEffect(() => {
    function handleMailShortcut(event: Event) {
      const customEvent = event as CustomEvent<{ action?: string }>;
      const action = customEvent.detail?.action;
      if (!action) {
        return;
      }

      const focusedThreadId = findFocusedThreadId();
      const selectedItem =
        items.find((item) => item.thread_id === focusedThreadId) ?? items[0];
      if (!selectedItem) {
        return;
      }

      const actionButton = document.querySelector<HTMLButtonElement>(
        `[data-hail-thread-id="${CSS.escape(selectedItem.thread_id)}"] [data-hail-shortcut-action="${action}"]`,
      );
      actionButton?.click();
    }

    window.addEventListener('hail:mail-shortcut', handleMailShortcut);
    return () => window.removeEventListener('hail:mail-shortcut', handleMailShortcut);
  }, [items]);

  const classifyPowerThrough = useClassifyThreadMutation(client);
  const setAsidePowerThrough = useSetAsideThreadMutation(client);
  const trashPowerThrough = useTrashThreadMutation(client);
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
        {selected.size > 0 ? (
          <BatchActionBar
            count={selected.size}
            onDeselectAll={() => setSelected(new Set())}
            onArchive={() =>
              batchActionWithToast(
                (threadId) => apiClient.archiveThread(threadId),
                `${selected.size} thread${selected.size === 1 ? '' : 's'} archived.`,
              )
            }
            onTrash={() =>
              batchActionWithToast(
                (threadId) => apiClient.trashThread(threadId),
                `${selected.size} thread${selected.size === 1 ? '' : 's'} moved to trash.`,
              )
            }
            onSetAside={() =>
              batchActionWithToast(
                (threadId) => apiClient.setAsideThread(threadId),
                `${selected.size} thread${selected.size === 1 ? '' : 's'} added to Set Aside.`,
              )
            }
            onReplyLater={() =>
              batchActionWithToast(
                (threadId) => apiClient.replyLaterThread(threadId),
                `${selected.size} thread${selected.size === 1 ? '' : 's'} added to Reply Later.`,
              )
            }
          />
        ) : null}
        {view === 'imbox' && isSectionedImboxData(query.data) ? (
          <ImboxSectionedList
            data={query.data}
            client={client}
            selected={selected}
            powerThrough={powerThrough}
            ptIndex={ptIndex}
            powerThroughStatus={powerThroughStatus}
            onToggleSelect={toggleSelect}
            onStartPowerThrough={startPowerThrough}
            onPowerThroughAction={handlePowerThroughAction}
            onExitPowerThrough={exitPowerThrough}
          />
        ) : (
          <ListView
            items={getFlatViewItems(query.data)}
            renderItem={(item) => (
              <MailListRow
                item={item}
                view={view}
                client={client}
                selected={selected.has(item.thread_id)}
                onToggleSelect={() => toggleSelect(item.thread_id)}
              />
            )}
            keyExtractor={(item) => `${item.thread_id}:${item.email_id}`}
            hasMore={false}
            isLoadingMore={false}
            onLoadMore={() => {}}
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
