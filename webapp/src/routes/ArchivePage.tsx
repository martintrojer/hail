import { useMemo } from 'react';
import { type HailApiClient, type MailViewItem } from '../api/client';
import { useArchiveView, useClassifyThreadMutation } from '../api/query';
import { ErrorState } from '../components/ErrorState';
import { LoadingState } from '../components/LoadingState';
import { StateCard } from '../components/StateCard';
import { ThreadLink } from '../components/ThreadLink';
import { ListView } from '../components/ListView';
import { MailRow } from '../components/MailRow';
import { useUndoToast } from '../components/UndoToastProvider';
import { AppShell } from '../layout/AppShell';
import { pillButtonClass } from '../lib/buttonStyles';
import { actionErrorMessage, viewErrorMessage } from '../lib/errorMessages';

interface ArchivePageProps {
  client?: HailApiClient;
}

function ArchiveRow({
  item,
  client,
}: {
  item: MailViewItem;
  client?: HailApiClient;
}) {
  const undoToast = useUndoToast();
  const restore = useClassifyThreadMutation(client, {
    onSuccess: (data) => {
      undoToast.showToast({
        message: 'Thread restored to Imbox.',
        undo: data.undo ? { id: data.undo.id } : null,
        undoSuccessMessage: 'Restore undone.',
      });
    },
  });

  return (
    <div className="border-b border-border-hairline py-4 pl-3 pr-0 hover:bg-bg-hover sm:py-5">
      <div className="flex items-start justify-between gap-4">
        <ThreadLink
          threadId={item.thread_id}
          mailListItem
          className="min-w-0 flex-1 rounded-sm focus-ring outline-none"
          ariaLabel={`Open ${item.subject || 'thread'} from ${item.from || 'unknown sender'}`}
        >
          <MailRow
            from={item.from || 'Unknown sender'}
            subject={item.subject || '(no subject)'}
            preview={item.preview || 'No preview available.'}
            receivedAt={item.received_at}
            hasNotes={item.has_notes}
          />
        </ThreadLink>

        <div className="flex shrink-0 flex-col items-end gap-2 sm:flex-row sm:items-center">
          <button
            type="button"
            disabled={restore.isPending}
            onClick={() => restore.mutate({ threadId: item.thread_id, to: 'imbox' })}
            className={pillButtonClass('primary')}
          >
            Restore
          </button>
        </div>
      </div>

      {restore.error ? (
        <p role="alert" className="mt-2 text-sm text-accent-red">
          {actionErrorMessage(restore.error, 'Thread action')}
        </p>
      ) : null}
    </div>
  );
}

export function ArchivePage({ client }: ArchivePageProps) {
  const query = useArchiveView(client);
  const items = useMemo(
    () => (query.isSuccess ? query.data.items : []),
    [query.data?.items, query.isSuccess],
  );

  let list;
  if (query.isPending) {
    list = <LoadingState label="Loading Archive mail" />;
  } else if (query.isError) {
    list = (
      <ErrorState
        message={viewErrorMessage(query.error, 'Archive')}
        onRetry={() => void query.refetch()}
      />
    );
  } else {
    list = (
      <ListView
        items={items}
        renderItem={(item) => <ArchiveRow item={item} client={client} />}
        keyExtractor={(item) => `${item.thread_id}:${item.email_id}`}
        hasMore={false}
        isLoadingMore={false}
        onLoadMore={() => {}}
        emptyState={<StateCard title="Nothing archived yet." />}
      />
    );
  }

  return (
    <AppShell
      title="Archive"
      description="Mail you have dealt with and moved out of the Imbox."
      list={list}
    />
  );
}
