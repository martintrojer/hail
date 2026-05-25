import { useNavigate } from '@tanstack/react-router';
import { useEffect, useMemo, useState } from 'react';
import {
  type HailApiClient,

  type MailViewItem,
  type MailViewKind,
} from '../api/client';
import {
  useArchiveThreadMutation,
  useClassifyThreadMutation,
  useFeedView,
  useImboxView,
  usePapertrailView,
  useReplyLaterThreadMutation,
  useScreenerView,
  useSetAsideThreadMutation,
  useTrashThreadMutation,
} from '../api/query';
import { ErrorState } from '../components/ErrorState';
import { ArrowUpCircle, X, iconSizeProps } from '../components/icons';
import { LoadingState } from '../components/LoadingState';
import { StateCard } from '../components/StateCard';
import { ThreadLink } from '../components/ThreadLink';
import { ListView } from '../components/ListView';
import { MailRow as SharedMailRow } from '../components/MailRow';
import { ScreenerBanner } from '../components/ScreenerBanner';
import { useUndoToast } from '../components/UndoToastProvider';
import { AppShell } from '../layout/AppShell';
import { pillButtonClass } from '../lib/buttonStyles';
import { formatDateTime } from '../lib/dates';
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
      return useImboxView(client);
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
}: {
  item: MailViewItem;
  view: MailViewKind;
  client?: HailApiClient;
}) {
  return (
    <div className="relative">
      <MailThreadRow item={item} view={view} />
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

function MailThreadRow({ item, view }: { item: MailViewItem; view: MailViewKind }) {
  if (view === 'feed') {
    return <FeedThreadRow item={item} />;
  }

  if (view === 'papertrail') {
    return <PaperTrailThreadRow item={item} />;
  }

  return <ImboxThreadRow item={item} />;
}

function ImboxThreadRow({ item }: { item: MailViewItem }) {
  return (
    <ThreadLink
      threadId={item.thread_id}
      mailListItem
      className="block border-b border-l-[3px] border-b-border-hairline border-l-transparent py-4 pl-3 pr-0 hover:bg-bg-hover focus-visible:border-l-accent-blue focus-visible:bg-bg-selected focus-visible:outline-none sm:py-5"
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
      />
    </ThreadLink>
  );
}

function FeedThreadRow({ item }: { item: MailViewItem }) {
  return (
    <ThreadLink
      threadId={item.thread_id}
      mailListItem
      className="block border-b border-l-[3px] border-b-border-hairline border-l-transparent py-6 pl-3 pr-0 hover:bg-bg-hover focus-visible:border-l-accent-blue focus-visible:bg-bg-selected focus-visible:outline-none sm:py-7"
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
      />
    </ThreadLink>
  );
}

function PaperTrailThreadRow({ item }: { item: MailViewItem }) {
  return (
    <ThreadLink
      threadId={item.thread_id}
      mailListItem
      className="block border-b border-l-[3px] border-b-border-hairline border-l-transparent py-2.5 pl-3 pr-0 hover:bg-bg-hover focus-visible:border-l-accent-blue focus-visible:bg-bg-selected focus-visible:outline-none sm:py-3"
      ariaLabel={`Open ${item.subject || 'thread'} from ${item.from || 'unknown sender'}`}
    >
      <ScreenReaderThreadMetadata item={item} />
      <SharedMailRow
        from={item.from || 'Unknown sender'}
        subject={item.subject || '(no subject)'}
        preview=""
        receivedAt={item.received_at}
        hasNotes={item.has_notes}
      />
    </ThreadLink>
  );
}

function PowerThroughMode({
  items,
  client,
  onDone,
}: {
  items: MailViewItem[];
  client?: HailApiClient;
  onDone: () => void;
}) {
  const undoToast = useUndoToast();
  const [currentIndex, setCurrentIndex] = useState(0);
  const currentItem = items[currentIndex];
  const remainingCount = Math.max(items.length - currentIndex - 1, 0);

  function advance() {
    setCurrentIndex((index) => Math.min(index + 1, items.length));
  }

  const classify = useClassifyThreadMutation(client, {
    onSuccess: (data, variables) => {
      const label = viewLabels[variables.to];
      undoToast.showToast({
        message: `Moved thread to ${label}.`,
        undo: data.undo ? { id: data.undo.id } : null,
        undoSuccessMessage: 'Thread classification undone.',
      });
      advance();
    },
  });
  const setAside = useSetAsideThreadMutation(client, {
    onSuccess: (data) => {
      undoToast.showToast({
        message: 'Thread added to Set Aside.',
        undo: data.undo ? { id: data.undo.id } : null,
        undoSuccessMessage: 'Set Aside undone.',
      });
      advance();
    },
  });
  const replyLater = useReplyLaterThreadMutation(client, {
    onSuccess: (data) => {
      undoToast.showToast({
        message: 'Thread added to Reply Later.',
        undo: data.undo ? { id: data.undo.id } : null,
        undoSuccessMessage: 'Reply Later undone.',
      });
      advance();
    },
  });
  const navigate = useNavigate();

  const busy = classify.isPending || setAside.isPending || replyLater.isPending;
  const error = classify.error ?? setAside.error ?? replyLater.error;

  function classifyCurrent(to: MailViewKind) {
    if (!currentItem) {
      return;
    }
    classify.mutate({ threadId: currentItem.thread_id, to });
  }

  function setCurrentAside() {
    if (!currentItem) {
      return;
    }
    setAside.mutate({ threadId: currentItem.thread_id });
  }

  function setCurrentReplyLater() {
    if (!currentItem) {
      return;
    }
    replyLater.mutate({ threadId: currentItem.thread_id });
  }

  function replyCurrent() {
    if (!currentItem) {
      return;
    }
    void navigate({ to: '/compose', search: { replyTo: currentItem.thread_id, replyAll: false } });
  }

  if (!currentItem) {
    return (
      <StateCard
        title="Power through complete"
        body="You made it through every Imbox thread in this batch."
      />
    );
  }

  return (
    <section
      className="border-y border-border-hairline py-6"
      aria-label="Power through Imbox"
    >
      <div className="mb-5 flex items-center justify-between gap-4">
        <div>
          <p className="text-sm font-semibold text-ink-secondary">Power through</p>
          <p className="mt-1 text-sm text-ink-tertiary">
            {remainingCount} thread{remainingCount === 1 ? '' : 's'} after this one
          </p>
        </div>
        <button
          type="button"
          onClick={onDone}
          className="inline-flex items-center gap-2 rounded-md px-2 py-1 text-sm font-semibold text-ink-secondary focus-ring outline-none hover:bg-bg-hover hover:text-ink-primary"
        >
          <X {...iconSizeProps.sm} aria-hidden="true" />
          Done
        </button>
      </div>

      <article className="rounded-lg bg-bg-surface p-6 shadow-lg shadow-ink-primary/10">
        <div className="flex items-baseline justify-between gap-4">
          <div className="min-w-0">
            <p className="truncate text-lg font-bold text-ink-primary">
              {currentItem.from || 'Unknown sender'}
            </p>
            <h2 className="mt-2 text-2xl font-bold leading-tight text-ink-primary">
              {currentItem.subject || '(no subject)'}
            </h2>
          </div>
          <time className="shrink-0 text-sm text-ink-tertiary">
            {formatDateTime(currentItem.received_at)}
          </time>
        </div>
        <p className="mt-4 text-base leading-7 text-ink-secondary">
          {currentItem.preview || 'No preview available.'}
        </p>

        <div className="mt-6 flex flex-wrap gap-2">
          <button
            type="button"
            disabled={busy}
            onClick={() => classifyCurrent('imbox')}
            className={pillButtonClass('primary')}
          >
            Keep in Imbox
          </button>
          <button
            type="button"
            disabled={busy}
            onClick={() => classifyCurrent('feed')}
            className={pillButtonClass('outline')}
          >
            Move to Feed
          </button>
          <button
            type="button"
            disabled={busy}
            onClick={() => classifyCurrent('papertrail')}
            className={pillButtonClass('outline')}
          >
            Move to Paper Trail
          </button>
          <button
            type="button"
            disabled={busy}
            onClick={setCurrentAside}
            className={pillButtonClass('outline')}
          >
            Set Aside
          </button>
          <button
            type="button"
            disabled={busy}
            onClick={setCurrentReplyLater}
            className={pillButtonClass('outline')}
          >
            Reply Later
          </button>
          <button
            type="button"
            disabled={busy}
            onClick={replyCurrent}
            className={pillButtonClass('outline')}
          >
            Reply
          </button>
        </div>

        {error ? (
          <p role="alert" className="mt-4 text-sm text-accent-red">
            {actionErrorMessage(error, 'Thread action')}
          </p>
        ) : null}
      </article>
    </section>
  );
}

function PowerThroughButton({ onClick }: { onClick: () => void }) {
  return (
    <button
      type="button"
      onClick={onClick}
      className="inline-flex items-center gap-2 rounded-md px-2 py-1 text-sm font-semibold text-ink-secondary focus-ring outline-none hover:bg-bg-hover hover:text-ink-primary"
    >
      <ArrowUpCircle {...iconSizeProps.sm} aria-hidden="true" />
      Power through
    </button>
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
  const screenerQuery = useScreenerView(client);
  const pendingCount = screenerQuery.data?.senders?.length ?? 0;
  const [powerThroughOpen, setPowerThroughOpen] = useState(false);

  const items = useMemo(
    () => (query.isSuccess ? query.data.items : []),
    [query.data?.items, query.isSuccess],
  );

  useEffect(() => {
    if (view !== 'imbox') {
      setPowerThroughOpen(false);
    }
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
  } else if (view === 'imbox' && powerThroughOpen) {
    list = (
      <PowerThroughMode
        items={query.data.items}
        client={client}
        onDone={() => setPowerThroughOpen(false)}
      />
    );
  } else {
    const emptyState = emptyStates[view];
    list = (
      <div>
        {view === 'imbox' ? <ScreenerBanner pendingCount={pendingCount} /> : null}
        <ListView
          items={query.data.items}
          renderItem={(item) => <MailListRow item={item} view={view} client={client} />}
          keyExtractor={(item) => `${item.thread_id}:${item.email_id}`}
          hasMore={false}
          isLoadingMore={false}
          onLoadMore={() => {}}
          emptyState={<StateCard title={emptyState.title} body={emptyState.body} />}
        />
      </div>
    );
  }

  return (
    <AppShell
      title={title}
      description={description}
      actions={
        view === 'imbox' && !query.isPending && !query.isError && query.data.items.length > 0 ? (
          <PowerThroughButton onClick={() => setPowerThroughOpen(true)} />
        ) : undefined
      }
      list={list}
    />
  );
}
