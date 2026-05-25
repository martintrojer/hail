import { Link } from '@tanstack/react-router';
import { useMemo } from 'react';
import { HailApiError, type HailApiClient, type MailViewItem } from '../api/client';
import { useDestroyThreadMutation, useRestoreThreadMutation, useTrashView } from '../api/query';
import { ErrorState } from '../components/ErrorState';
import { LoadingState } from '../components/LoadingState';
import { ListView } from '../components/ListView';
import { useOptionalUndoToast } from '../components/UndoToastProvider';
import { AppShell } from '../layout/AppShell';

interface TrashPageProps {
  client?: HailApiClient;
}

function formatDate(value: string | null | undefined) {
  if (!value) {
    return 'No date';
  }

  const date = new Date(value);
  if (Number.isNaN(date.valueOf())) {
    return value;
  }

  const now = new Date();
  const sameYear = date.getFullYear() === now.getFullYear();
  return new Intl.DateTimeFormat(undefined, {
    month: 'short',
    day: 'numeric',
    ...(sameYear ? {} : { year: 'numeric' }),
    hour: 'numeric',
    minute: '2-digit',
  }).format(date);
}

function errorMessage(error: Error) {
  if (error instanceof HailApiError) {
    if (error.status === 401) {
      return 'Your session expired. Sign in again to refresh Trash.';
    }
    return `Trash failed with HTTP ${error.status}.`;
  }

  return 'Trash failed to load. Refresh and try again.';
}

function threadActionErrorMessage(error: Error) {
  if (error instanceof HailApiError) {
    return `Thread action failed with HTTP ${error.status}.`;
  }

  return 'Thread action failed. Try again.';
}

function EmptyTrash() {
  return (
    <div className="flex min-h-[300px] flex-col items-center justify-center p-8 text-center">
      <p className="text-lg font-semibold text-ink-primary">Trash is empty.</p>
    </div>
  );
}

function TrashRow({
  item,
  client,
}: {
  item: MailViewItem;
  client?: HailApiClient;
}) {
  const undoToast = useOptionalUndoToast();
  const restore = useRestoreThreadMutation(client, {
    onSuccess: (data) => {
      undoToast?.showToast({
        message: 'Thread restored to Imbox.',
        undo: data.undo ? { id: data.undo.id } : null,
        undoSuccessMessage: 'Restore undone.',
      });
    },
  });
  const destroy = useDestroyThreadMutation(client);
  const isMutating = restore.isPending || destroy.isPending;

  return (
    <div className="border-b border-border-hairline py-4 pl-3 pr-0 hover:bg-bg-hover sm:py-5">
      <div className="flex items-start justify-between gap-4">
        <Link
          to="/thread/$threadId"
          search={{ from: undefined }} params={{ threadId: item.thread_id }}
          className="min-w-0 flex-1 rounded-sm outline-none focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent-blue"
          data-hail-mail-list-item="true"
          data-hail-thread-id={item.thread_id}
          aria-label={`Open ${item.subject || 'thread'} from ${item.from || 'unknown sender'}`}
        >
          <div className="flex items-baseline justify-between gap-4">
            <p className="truncate text-base font-semibold leading-snug text-ink-primary">
              {item.from || 'Unknown sender'}
            </p>
            <time className="shrink-0 text-sm leading-snug text-ink-tertiary">
              {formatDate(item.received_at)}
            </time>
          </div>
          <p className="mt-1 truncate text-[0.95rem] font-normal leading-snug text-ink-secondary">
            {item.subject || '(no subject)'}
          </p>
          <p className="mt-1 truncate text-sm font-normal leading-snug text-ink-tertiary">
            {item.preview || 'No preview available.'}
          </p>
        </Link>

        <div className="flex shrink-0 flex-col items-end gap-2 sm:flex-row sm:items-center">
          <button
            type="button"
            disabled={isMutating}
            onClick={() => restore.mutate({ threadId: item.thread_id })}
            className="rounded-full bg-accent-blue px-3 py-1 text-xs font-semibold text-white outline-none hover:bg-accent-blue-hover focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent-blue disabled:cursor-not-allowed disabled:opacity-60"
          >
            Restore
          </button>
          <button
            type="button"
            disabled={isMutating}
            onClick={() => destroy.mutate({ threadId: item.thread_id })}
            className="rounded-full border border-border-menu px-3 py-1 text-xs font-semibold text-ink-tertiary outline-none hover:bg-bg-hover hover:text-accent-red focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent-blue disabled:cursor-not-allowed disabled:opacity-60"
          >
            Delete forever
          </button>
        </div>
      </div>

      {restore.error || destroy.error ? (
        <p role="alert" className="mt-2 text-sm text-accent-red">
          {threadActionErrorMessage(restore.error ?? destroy.error!)}
        </p>
      ) : null}
    </div>
  );
}

export function TrashPage({ client }: TrashPageProps) {
  const query = useTrashView(client);
  const items = useMemo(
    () => (query.isSuccess ? query.data.items : []),
    [query.data?.items, query.isSuccess],
  );

  let list;
  if (query.isPending) {
    list = <LoadingState label="Loading Trash mail" />;
  } else if (query.isError) {
    list = (
      <ErrorState
        message={errorMessage(query.error)}
        onRetry={() => void query.refetch()}
      />
    );
  } else {
    list = (
      <ListView
        items={items}
        renderItem={(item) => <TrashRow item={item} client={client} />}
        keyExtractor={(item) => `${item.thread_id}:${item.email_id}`}
        hasMore={false}
        isLoadingMore={false}
        onLoadMore={() => {}}
        emptyState={<EmptyTrash />}
      />
    );
  }

  return (
    <AppShell
      title="Trash"
      description="Deleted mail stays here until it is permanently removed."
      list={list}
    />
  );
}
