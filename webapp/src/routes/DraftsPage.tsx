import { Link } from '@tanstack/react-router';
import { useMemo } from 'react';
import {
  type HailApiClient,
  type MailViewItem,
} from '../api/client';
import { defaultApiClient, useDeleteDraftMutation, useDraftsView } from '../api/query';
import { ErrorState } from '../components/ErrorState';
import { Trash2, iconSizeProps } from '../components/icons';
import { LoadingState } from '../components/LoadingState';
import { StateCard } from '../components/StateCard';
import { ListView } from '../components/ListView';
import { MailRow } from '../components/MailRow';
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
    <button
      type="button"
      onClick={(event) => {
        event.preventDefault();
        event.stopPropagation();
        deleteDraft.mutate(draftId);
      }}
      disabled={deleteDraft.isPending}
      className="rounded-full p-2 text-ink-tertiary focus-ring outline-none hover:bg-bg-hover hover:text-accent-red disabled:cursor-not-allowed disabled:opacity-60"
      aria-label="Delete draft"
      title={deleteDraft.error ? actionErrorMessage(deleteDraft.error) : 'Delete draft'}
    >
      <Trash2 {...iconSizeProps.sm} aria-hidden="true" />
    </button>
  );
}

function DraftRow({ item, client }: { item: MailViewItem; client: HailApiClient }) {
  return (
    <div className="relative flex items-center gap-3 border-b border-border-hairline py-4 pl-3 pr-0 hover:bg-bg-hover sm:py-5">
      <Link
        to="/compose"
        search={{ draftId: item.email_id }}
        className="min-w-0 flex-1 focus-ring outline-none focus-visible:rounded-md"
        data-hail-mail-list-item="true"
        aria-label={`Resume draft ${item.subject || '(no subject)'}`}
      >
        <MailRow
          from={item.subject || '(no subject)'}
          subject={recipientSummary(item)}
          preview=""
          receivedAt={item.received_at}
          receivedAtFallback="Not saved yet"
        />
      </Link>
      <DeleteDraftButton draftId={item.email_id} client={client} />
    </div>
  );
}

export function DraftsPage({ client = defaultApiClient }: DraftsPageProps) {
  const query = useDraftsView(client);
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
      <ListView
        items={items}
        renderItem={(item) => <DraftRow item={item} client={client} />}
        keyExtractor={(item) => `${item.thread_id}:${item.email_id}`}
        hasMore={false}
        isLoadingMore={false}
        onLoadMore={() => {}}
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
