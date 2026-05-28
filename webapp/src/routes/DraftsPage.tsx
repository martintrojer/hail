import { Link } from '@tanstack/react-router';
import { useMemo } from 'react';
import { useApiClient } from '../api/ApiClientProvider';
import {
  type HailApiClient,
  type MailViewItem,
} from '../api/client';
import { useDeleteDraftMutation, useDraftsView } from '../api/query';
import { ActionableList } from '../components/ActionableList';
import { Trash2 } from '../components/icons';
import { LoadingState } from '../components/LoadingState';
import { ErrorState } from '../components/ErrorState';
import { StateCard } from '../components/StateCard';
import { MailRow } from '../components/MailRow';
import { Button } from '../components/ui/button';
import { AppShell } from '../layout/AppShell';
import { actionErrorMessage, viewErrorMessage } from '../lib/errorMessages';

interface DraftsPageProps {
  client?: HailApiClient;
}

function recipientSummary(item: MailViewItem) {
  return item.from || 'No recipients';
}

function DeleteDraftButton({ draftId, client }: { draftId: string; client: HailApiClient }) {
  const deleteDraft = useDeleteDraftMutation(client);

  return (
    <Button
      type="button"
      onClick={(event) => {
        event.preventDefault();
        event.stopPropagation();
        deleteDraft.mutate(draftId);
      }}
      disabled={deleteDraft.isPending}
      variant="ghost"
      size="icon-sm"
      aria-label="Delete draft"
      title={deleteDraft.error ? actionErrorMessage(deleteDraft.error) : 'Delete draft'}
    >
      <Trash2 aria-hidden="true" />
    </Button>
  );
}

function DraftRow({ item, client }: { item: MailViewItem; client: HailApiClient }) {
  return (
    <div className="relative flex items-center gap-3 border-b border-border-hairline py-4 pl-3 pr-0 hover:bg-bg-hover sm:py-5">
      <Link
        to="/compose"
        search={{ draftId: item.email_id }}
        className="min-w-0 flex-1 focus-ring outline-none focus-visible:rounded-md"
        data-hail-thread-id={item.email_id}
        data-hail-mail-list-item="true"
        aria-label={`Resume draft ${item.subject || '(no subject)'}`}
      >
        <MailRow
          from={item.subject || '(no subject)'}
          subject={recipientSummary(item)}
          preview=""
          receivedAt={item.received_at}
          receivedAtFallback="Not saved yet"
          hasNotes={item.has_notes}
        />
      </Link>
      <DeleteDraftButton draftId={item.email_id} client={client} />
    </div>
  );
}

export function DraftsPage({ client }: DraftsPageProps) {
  const contextClient = useApiClient();
  const apiClient = client ?? contextClient;
  const query = useDraftsView(apiClient);
  const items = useMemo(
    () => (query.isSuccess ? query.data.items : []),
    [query.data?.items, query.isSuccess],
  );

  let list;
  if (query.isPending) {
    list = <LoadingState label="Loading drafts" />;
  } else if (query.isError) {
    list = (
      <ErrorState
        message={viewErrorMessage(query.error, 'Drafts')}
        onRetry={() => void query.refetch()}
      />
    );
  } else {
    list = (
      <ActionableList
        items={items}
        actions={{ client: apiClient, availableActions: ['delete'] }}
        renderItem={(item) => <DraftRow item={item} client={apiClient} />}
        keyExtractor={(item) => item.email_id}
        emptyState={<StateCard title="No drafts." />}
      />
    );
  }

  return (
    <AppShell
      title="Drafts"
      description="Resume messages you started but have not sent yet."
      list={list}
    />
  );
}
