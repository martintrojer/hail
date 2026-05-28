import { useMemo } from 'react';
import { type HailApiClient, type MailViewItem } from '../api/client';
import { useTrashView } from '../api/query';
import { ActionableList } from '../components/ActionableList';
import { ErrorState } from '../components/ErrorState';
import { LoadingState } from '../components/LoadingState';
import { StateCard } from '../components/StateCard';
import { ThreadLink } from '../components/ThreadLink';
import { MailRow } from '../components/MailRow';
import { AppShell } from '../layout/AppShell';
import { viewErrorMessage } from '../lib/errorMessages';

interface TrashPageProps {
  client?: HailApiClient;
}

function TrashRow({
  item,
  selected,
  onToggleSelect,
}: {
  item: MailViewItem;
  selected?: boolean;
  onToggleSelect?: () => void;
}) {
  return (
    <div className="border-b border-border py-4 pl-3 pr-0 hover:bg-muted/50 sm:py-5">
      <ThreadLink
        threadId={item.thread_id}
        mailListItem
        className="block min-w-0 rounded-sm focus-ring outline-none"
        ariaLabel={`Open ${item.subject || 'thread'} from ${item.from || 'unknown sender'}`}
      >
        <MailRow
          from={item.from || 'Unknown sender'}
          subject={item.subject || '(no subject)'}
          preview={item.preview || 'No preview available.'}
          receivedAt={item.received_at}
          hasNotes={item.has_notes}
          selected={selected}
          onToggleSelect={onToggleSelect}
        />
      </ThreadLink>
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
        message={viewErrorMessage(query.error, 'Trash')}
        onRetry={() => void query.refetch()}
      />
    );
  } else {
    list = (
      <ActionableList
        items={items}
        actions={{ client, availableActions: ['restore', 'delete-forever'], restoreMode: 'restore-endpoint' }}
        renderItem={(item, { selected, onToggleSelect }) => (
          <TrashRow item={item} selected={selected} onToggleSelect={onToggleSelect} />
        )}
        keyExtractor={(item) => item.thread_id}
        emptyState={<StateCard title="Trash is empty." />}
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
