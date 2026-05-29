import { useMemo } from 'react';
import { type HailApiClient } from '../api/client';
import { useArchiveView } from '../api/query';
import { ActionableList } from '../components/ActionableList';
import { ErrorState } from '../components/ErrorState';
import { LoadingState } from '../components/LoadingState';
import { MailThreadRow } from '../components/MailThreadRow';
import { StateCard } from '../components/StateCard';
import { AppShell } from '../layout/AppShell';
import { viewErrorMessage } from '../lib/errorMessages';

interface ArchivePageProps {
  client?: HailApiClient;
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
      <ActionableList
        items={items}
        actions={{ client, availableActions: ['restore'], restoreMode: 'classify-imbox' }}
        renderItem={(item, { selected, onToggleSelect }) => (
          <MailThreadRow item={item} selected={selected} onToggleSelect={onToggleSelect} />
        )}
        keyExtractor={(item) => item.thread_id}
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
