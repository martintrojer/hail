import { useMemo } from 'react';
import { type HailApiClient, type MailViewItem } from '../api/client';
import { useSpamView } from '../api/query';
import { ActionableList } from '../components/ActionableList';
import { ErrorState } from '../components/ErrorState';
import { LoadingState } from '../components/LoadingState';
import { MailRow } from '../components/MailRow';
import { StateCard } from '../components/StateCard';
import { ThreadLink } from '../components/ThreadLink';
import { AppShell } from '../layout/AppShell';
import { viewErrorMessage } from '../lib/errorMessages';

interface SpamPageProps {
  client?: HailApiClient;
}

function SpamRow({
  item,
  selected,
  onToggleSelect,
}: {
  item: MailViewItem;
  selected?: boolean;
  onToggleSelect?: () => void;
}) {
  return (
    <div className="border-b border-border-hairline py-4 pl-3 pr-0 hover:bg-bg-hover sm:py-5">
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
          <SpamRow item={item} selected={selected} onToggleSelect={onToggleSelect} />
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
