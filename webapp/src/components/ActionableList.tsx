import { useEffect, useMemo, useState, type ReactNode } from 'react';
import type { HailApiClient } from '../api/client';
import { actionErrorMessage } from '../lib/errorMessages';
import { BatchActionBar } from './BatchActionBar';
import { ListView } from './ListView';
import { useListActions, type ListActionConfig, type ListAction } from '../hooks/useListActions';

export interface ActionableListProps<T> {
  items: T[];
  keyExtractor: (item: T) => string;
  renderItem: (
    item: T,
    props: { selected: boolean; onToggleSelect: () => void },
  ) => ReactNode;
  actions: ListActionConfig;
  client?: HailApiClient;
  emptyState?: ReactNode;
  hasMore?: boolean;
  isLoadingMore?: boolean;
  onLoadMore?: () => void;
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

const labels: Record<ListAction, string> = {
  archive: 'Archive',
  trash: 'Trash',
  'set-aside': 'Set Aside',
  'reply-later': 'Reply Later',
  classify: 'Classify',
  restore: 'Restore',
  delete: 'Delete',
  'delete-forever': 'Delete forever',
};

function shortcutAction(action: ListAction) {
  if (action === 'restore') {
    return 'archive';
  }

  if (action === 'delete-forever' || action === 'delete') {
    return 'trash';
  }

  if (action === 'classify') {
    return 'archive';
  }

  return action;
}

function handlerName(action: ListAction) {
  if (action === 'set-aside') return 'setAside';
  if (action === 'reply-later') return 'replyLater';
  if (action === 'delete-forever') return 'deleteForever';
  return action;
}

function findFocusedThreadId() {
  if (!(document.activeElement instanceof HTMLElement)) {
    return null;
  }

  return document.activeElement.dataset.hailThreadId ?? null;
}

export function ActionableList<T>({
  items,
  keyExtractor,
  renderItem,
  actions,
  client,
  emptyState = null,
  hasMore = false,
  isLoadingMore = false,
  onLoadMore = () => {},
}: ActionableListProps<T>) {
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const listActions = useListActions({ ...actions, client: actions.client ?? client });
  const itemIds = useMemo(() => items.map(keyExtractor), [items, keyExtractor]);
  const itemIdSet = useMemo(() => new Set(itemIds), [itemIds]);
  const availableActions = actions.availableActions;

  useEffect(() => {
    setSelected((prev) => {
      const validIds = new Set(itemIds);
      const next = new Set([...prev].filter((id) => validIds.has(id)));
      return next.size === prev.size ? prev : next;
    });
  }, [itemIds]);

  function toggleSelect(id: string) {
    setSelected((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  }

  function onBatch(action: ListAction) {
    const threadIds = [...selected];
    void listActions.runBatch(action, threadIds).then(() => setSelected(new Set()));
  }

  useEffect(() => {
    function handleMailShortcut(event: Event) {
      const customEvent = event as CustomEvent<{ action?: string }>;
      const action = customEvent.detail?.action;
      if (!action) {
        return;
      }

      const focusedThreadId = findFocusedThreadId();
      const firstVisibleItem = document.querySelector<HTMLElement>(
        '[data-hail-mail-list-item="true"]',
      );
      const selectedThreadId = focusedThreadId ?? firstVisibleItem?.dataset.hailThreadId ?? null;
      if (!selectedThreadId || !itemIdSet.has(selectedThreadId)) {
        return;
      }

      const actionButton = document.querySelector<HTMLButtonElement>(
        `[data-hail-thread-id="${CSS.escape(selectedThreadId)}"] [data-hail-shortcut-action="${action}"]`,
      );
      actionButton?.click();
    }

    window.addEventListener('hail:mail-shortcut', handleMailShortcut);
    return () => window.removeEventListener('hail:mail-shortcut', handleMailShortcut);
  }, [itemIdSet]);

  return (
    <>
      {selected.size > 0 ? (
        <BatchActionBar
          count={selected.size}
          availableActions={availableActions}
          onDeselectAll={() => setSelected(new Set())}
          onAction={onBatch}
        />
      ) : null}
      <ListView
        items={items}
        keyExtractor={keyExtractor}
        hasMore={hasMore}
        isLoadingMore={isLoadingMore}
        onLoadMore={onLoadMore}
        emptyState={emptyState}
        renderItem={(item) => {
          const id = keyExtractor(item);
          return (
            <div
              data-hail-thread-id={id}
              className="relative"
            >
              {renderItem(item, {
                selected: selected.has(id),
                onToggleSelect: () => toggleSelect(id),
              })}
              {availableActions.map((action) => (
                <ShortcutActionButton
                  key={action}
                  action={shortcutAction(action)}
                  label={labels[action]}
                  busy={listActions.isBusy}
                  onClick={() => {
                    const run = listActions[handlerName(action) as keyof typeof listActions];
                    if (typeof run === 'function') {
                      void (run as (threadId: string) => Promise<unknown>)(id);
                    }
                  }}
                />
              ))}
              {listActions.error ? (
                <span role="alert" className="sr-only">
                  {actionErrorMessage(listActions.error, 'Thread action')}
                </span>
              ) : null}
            </div>
          );
        }}
      />
    </>
  );
}
