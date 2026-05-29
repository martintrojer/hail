import { useMemo } from 'react';
import { type HailApiClient } from '../api/client';
import { useTrashView } from '../api/query';
import { ActionableList } from '../components/ActionableList';
import { ErrorState } from '../components/ErrorState';
import { LoadingState } from '../components/LoadingState';
import { MailThreadRow } from '../components/MailThreadRow';
import { StateCard } from '../components/StateCard';
import { AppShell } from '../layout/AppShell';
import { viewErrorMessage } from '../lib/errorMessages';

interface TrashPageProps {
  client?: HailApiClient;
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
          <MailThreadRow item={item} selected={selected} onToggleSelect={onToggleSelect} />
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
