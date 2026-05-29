import { useMemo } from 'react';
import { type HailApiClient, type MailViewItem } from '../api/client';
import { useSpamView } from '../api/query';
import { ActionableList } from '../components/ActionableList';
import { ErrorState } from '../components/ErrorState';
import { LoadingState } from '../components/LoadingState';
import { MailThreadRow } from '../components/MailThreadRow';
import { StateCard } from '../components/StateCard';
import { AppShell } from '../layout/AppShell';
import { viewErrorMessage } from '../lib/errorMessages';

interface SpamPageProps {
  client?: HailApiClient;
}

export function SpamPage({ client }: SpamPageProps) {
  const query = useSpamView(client);
  const items = useMemo(
    () => (query.isSuccess ? query.data.items : []),
    [query.data?.items, query.isSuccess],
  );

  let list;
  if (query.isPending) {
    list = <LoadingState label="Loading Spam mail" />;
  } else if (query.isError) {
    list = (
      <ErrorState
        message={viewErrorMessage(query.error, 'Spam')}
        onRetry={() => void query.refetch()}
      />
    );
  } else {
    list = (
      <ActionableList
        items={items}
        actions={{ client, availableActions: ['not-spam', 'delete-forever'] }}
        renderItem={(item: MailViewItem, { selected, onToggleSelect }) => (
          <MailThreadRow item={item} selected={selected} onToggleSelect={onToggleSelect} />
        )}
        keyExtractor={(item) => item.thread_id}
        emptyState={<StateCard title="No spam. Nice." />}
      />
    );
  }

  return (
    <AppShell
      title="Spam"
      description="Mail identified as spam collects here until you restore or delete it."
      list={list}
    />
  );
}
