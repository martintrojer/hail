import { Link } from '@tanstack/react-router';
import { useMemo } from 'react';
import {
  HailApiError,
  type HailApiClient,
  type MailViewItem,
} from '../api/client';
import { defaultApiClient, useDeleteDraftMutation, useDraftsView } from '../api/query';
import { ErrorState } from '../components/ErrorState';
import { Trash2, iconSizeProps } from '../components/icons';
import { LoadingState } from '../components/LoadingState';
import { ListView } from '../components/ListView';
import { AppShell } from '../layout/AppShell';

interface DraftsPageProps {
  client?: HailApiClient;
}

function errorMessage(error: Error) {
  if (error instanceof HailApiError) {
    if (error.status === 401) {
      return 'Your session expired. Sign in again to refresh drafts.';
    }
    return `Drafts failed with HTTP ${error.status}.`;
  }

  return 'Drafts failed to load. Refresh and try again.';
}

function formatDate(value: string | null | undefined) {
  if (!value) {
    return 'Not saved yet';
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

function recipientSummary(item: MailViewItem) {
  return item.from || 'No recipients';
}

function StateCard({ title, body }: { title: string; body?: string }) {
  return (
    <div className="flex min-h-[300px] flex-col items-center justify-center p-8 text-center">
      <p className="text-lg font-semibold text-ink-primary">{title}</p>
      {body ? <p className="mt-2 max-w-sm text-sm leading-6 text-ink-secondary">{body}</p> : null}
    </div>
  );
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
      className="rounded-full p-2 text-ink-tertiary outline-none hover:bg-bg-hover hover:text-accent-red focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent-blue disabled:cursor-not-allowed disabled:opacity-60"
      aria-label="Delete draft"
      title={deleteDraft.error ? errorMessage(deleteDraft.error) : 'Delete draft'}
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
        className="min-w-0 flex-1 outline-none focus-visible:rounded-md focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent-blue"
        data-hail-mail-list-item="true"
        aria-label={`Resume draft ${item.subject || '(no subject)'}`}
      >
        <div className="flex items-baseline justify-between gap-4">
          <p className="truncate text-base font-semibold leading-snug text-ink-primary">
            {item.subject || '(no subject)'}
          </p>
          <time className="shrink-0 text-sm leading-snug text-ink-tertiary">
            {formatDate(item.received_at)}
          </time>
        </div>
        <p className="mt-1 truncate text-[0.95rem] font-normal leading-snug text-ink-secondary">
          {recipientSummary(item)}
        </p>
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
        message={errorMessage(query.error)}
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
