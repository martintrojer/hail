import { useRef, useState } from 'react';
import type { HailApiClient } from '../api/client';
import {
  HailApiError,
  type DeniedSender,
  type ScreenerClassification,
  type ScreenerPendingSender,
} from '../api/client';
import {
  useDeniedSenders,
  useScreenerDecisionMutation,
  useScreenerView,
  useUndoDenyMutation,
} from '../api/query';
import { ErrorState } from '../components/ErrorState';
import { LoadingState } from '../components/LoadingState';
import { ListView } from '../components/ListView';
import { ScreenerBanner } from '../components/ScreenerBanner';
import {
  ScreenerRoutingDropdown,
  type ScreenerRoutingDestination,
} from '../components/ScreenerRoutingDropdown';
import { useUndoToast } from '../components/UndoToastProvider';
import { AppShell } from '../layout/AppShell';

function errorMessage(error: Error) {
  if (error instanceof HailApiError) {
    if (error.status === 401) {
      return 'Your session expired. Sign in again to refresh the Screener.';
    }
    return `Screener failed with HTTP ${error.status}.`;
  }

  return 'Screener failed to load. Refresh and try again.';
}

function decisionErrorMessage(error: Error) {
  if (error instanceof HailApiError) {
    if (error.status === 400 || error.status === 422) {
      return 'The server rejected this decision. Refresh and try again.';
    }
    if (error.status === 401) {
      return 'Your session expired. Sign in again before deciding.';
    }
    return `Decision failed with HTTP ${error.status}.`;
  }

  return 'Decision failed. Try again.';
}

function previewRecord(preview: unknown) {
  if (!preview || typeof preview !== 'object') {
    return null;
  }

  return preview as Record<string, unknown>;
}

function textFromKeys(record: Record<string, unknown> | null, keys: string[]) {
  if (!record) {
    return null;
  }

  for (const key of keys) {
    const value = record[key];
    if (typeof value === 'string' && value.trim().length > 0) {
      return value.trim();
    }
  }

  return null;
}

function subjectText(preview: unknown) {
  return textFromKeys(previewRecord(preview), ['subject', 'title']);
}

function previewText(preview: unknown) {
  if (typeof preview === 'string' && preview.trim().length > 0) {
    return preview.trim();
  }

  return textFromKeys(previewRecord(preview), [
    'text',
    'body',
    'preview',
    'snippet',
    'summary',
  ]);
}

function formatPendingEmailDate(value?: string | null) {
  if (!value) {
    return 'Date unavailable';
  }

  const date = new Date(value);
  if (Number.isNaN(date.getTime())) {
    return value;
  }

  return new Intl.DateTimeFormat(undefined, {
    year: 'numeric',
    month: 'short',
    day: 'numeric',
  }).format(date);
}

function parseSender(sender: string) {
  const trimmed = sender.trim();
  const mailbox = trimmed.match(/^(.*?)\s*<([^>]+)>$/);
  if (mailbox) {
    const [, rawName, rawEmail] = mailbox;
    const name = rawName.replace(/^['"]|['"]$/g, '').trim();
    const email = rawEmail.trim();

    return {
      name: name || email,
      email,
    };
  }

  if (trimmed.includes('@')) {
    const localPart = trimmed.split('@')[0] ?? trimmed;
    const name = localPart
      .split(/[._+-]+/)
      .filter(Boolean)
      .map((part) => part.charAt(0).toUpperCase() + part.slice(1))
      .join(' ');

    return {
      name: name || trimmed,
      email: trimmed,
    };
  }

  return {
    name: trimmed || 'Unknown sender',
    email: trimmed || 'unknown address',
  };
}

function EmptyState() {
  return (
    <div className="flex min-h-[300px] flex-col items-center justify-center text-center">
      <p className="text-lg font-semibold text-ink-primary">
        All clear. No one new is waiting.
      </p>
      <span className="sr-only">No unknown senders</span>
    </div>
  );
}

function PendingSenderCard({
  sender,
  client,
}: {
  sender: ScreenerPendingSender;
  client?: HailApiClient;
}) {
  const [routingOpen, setRoutingOpen] = useState(false);
  const [routingAnchor, setRoutingAnchor] = useState<DOMRect | null>(null);
  const [expanded, setExpanded] = useState(false);
  const approveButtonRef = useRef<HTMLButtonElement | null>(null);
  const { showToast } = useUndoToast();
  const decision = useScreenerDecisionMutation(client, {
    onSuccess: (data, variables) => {
      if (variables.decision !== 'deny') {
        return;
      }

      showToast({
        message: `Denied ${variables.sender}.`,
        undo: data.undo ? { id: data.undo.id } : null,
        undoSuccessMessage: 'Sender decision undone.',
      });
    },
  });
  const isPending = decision.isPending;
  const senderIdentity = parseSender(sender.sender);
  const subject = subjectText(sender.latest_preview) ?? 'First message from this sender';
  const preview =
    previewText(sender.latest_preview) ??
    'Preview unavailable until this message is indexed.';
  const emails = sender.emails ?? [];
  const expandedId = `screener-emails-${encodeURIComponent(sender.sender)}`;
  const pendingEmailCount = sender.message_count ?? emails.length;
  const emailCountLabel = `${pendingEmailCount} pending ${pendingEmailCount === 1 ? 'email' : 'emails'}`;

  function showRoutingDropdown() {
    if (approveButtonRef.current) {
      setRoutingAnchor(approveButtonRef.current.getBoundingClientRect());
    }
    setRoutingOpen(true);
  }

  function approve(destination: ScreenerRoutingDestination) {
    decision.mutate({
      sender: sender.sender,
      decision: 'approve',
      classify_as: destination as ScreenerClassification,
      apply_to_history: true,
    });
  }

  function deny() {
    decision.mutate({
      sender: sender.sender,
      decision: 'deny',
      apply_to_history: true,
    });
  }

  return (
    <article className="rounded-lg bg-bg-surface p-5">
      <button
        type="button"
        onClick={() => setExpanded((value) => !value)}
        aria-expanded={expanded}
        aria-controls={expandedId}
        className="block w-full rounded-md text-left focus:outline-none focus:ring-2 focus:ring-accent-blue focus:ring-offset-2 focus:ring-offset-bg-surface"
      >
        <div className="flex items-start justify-between gap-4">
          <div className="min-w-0">
            <h2 className="hail-sender truncate text-ink-primary">
              {senderIdentity.name}
            </h2>
            <p className="mt-1 truncate text-sm text-ink-secondary">
              {senderIdentity.email}
            </p>
          </div>
          <span className="shrink-0 rounded-full border border-border-menu px-3 py-1 text-xs font-semibold text-ink-secondary">
            {expanded ? 'Hide' : 'Show'} · {emailCountLabel}
          </span>
        </div>

        <div className="mt-5 space-y-2">
          <p className="text-[0.95rem] leading-6 text-ink-secondary">{subject}</p>
          <p className="line-clamp-2 text-sm leading-6 text-ink-tertiary">
            {preview}
          </p>
        </div>
      </button>

      {expanded ? (
        <div id={expandedId} className="mt-5 border-t border-border-subtle pt-4">
          {emails.length === 0 ? (
            <p className="text-sm text-ink-tertiary">
              Pending email details are unavailable right now.
            </p>
          ) : (
            <ul className="space-y-3">
              {emails.map((email) => (
                <li
                  key={email.email_id}
                  className="rounded-md border border-border-subtle bg-bg-canvas/50 px-4 py-3"
                >
                  <div className="flex flex-col gap-1 sm:flex-row sm:items-start sm:justify-between sm:gap-4">
                    <p className="min-w-0 text-sm font-semibold text-ink-primary">
                      {email.subject || 'No subject'}
                    </p>
                    <time
                      dateTime={email.received_at ?? undefined}
                      className="shrink-0 text-xs text-ink-tertiary"
                    >
                      {formatPendingEmailDate(email.received_at)}
                    </time>
                  </div>
                  <p className="mt-2 line-clamp-2 text-sm leading-6 text-ink-tertiary">
                    {email.preview || 'Preview unavailable.'}
                  </p>
                </li>
              ))}
            </ul>
          )}
        </div>
      ) : null}

      <div className="mt-5 flex flex-wrap gap-3">
        <button
          ref={approveButtonRef}
          type="button"
          aria-label={isPending ? 'Saving…' : 'Approve'}
          aria-haspopup="menu"
          aria-expanded={routingOpen}
          onClick={showRoutingDropdown}
          disabled={isPending}
          className="rounded-full bg-accent-blue px-4 py-1.5 text-xs font-semibold text-white hover:bg-accent-blue-hover disabled:cursor-not-allowed disabled:opacity-60"
        >
          {isPending ? 'Saving…' : 'Yes'}
        </button>
        <button
          type="button"
          aria-label="Deny"
          onClick={deny}
          disabled={isPending}
          className="rounded-full border border-border-menu px-4 py-1.5 text-xs font-semibold text-ink-secondary hover:bg-bg-hover hover:text-ink-primary disabled:cursor-not-allowed disabled:opacity-60"
        >
          No
        </button>
      </div>

      <ScreenerRoutingDropdown
        open={routingOpen}
        anchorRect={routingAnchor}
        onClose={() => setRoutingOpen(false)}
        onSelect={approve}
      />

      {decision.isError ? (
        <p role="alert" className="mt-4 text-sm text-accent-red">
          {decisionErrorMessage(decision.error)}
        </p>
      ) : null}
    </article>
  );
}

function formatDeniedDate(value: string) {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) {
    return value;
  }

  return new Intl.DateTimeFormat(undefined, {
    year: 'numeric',
    month: 'short',
    day: 'numeric',
  }).format(date);
}

function DeniedSenderRow({
  sender,
  client,
}: {
  sender: DeniedSender;
  client?: HailApiClient;
}) {
  const { showToast } = useUndoToast();
  const undo = useUndoDenyMutation(client, {
    onSuccess: () => {
      showToast({ message: `Restored ${sender.sender_address} to the Screener.` });
    },
  });

  return (
    <div className="flex flex-col gap-3 rounded-lg border border-border-subtle bg-bg-surface px-4 py-3 sm:flex-row sm:items-center sm:justify-between">
      <div className="min-w-0">
        <p className="truncate text-sm font-semibold text-ink-primary">
          {sender.sender_address}
        </p>
        <p className="mt-1 text-xs text-ink-tertiary">
          Denied {formatDeniedDate(sender.denied_at)}
        </p>
        {undo.isError ? (
          <p role="alert" className="mt-2 text-xs text-accent-red">
            {decisionErrorMessage(undo.error)}
          </p>
        ) : null}
      </div>
      <button
        type="button"
        onClick={() => undo.mutate(sender.sender_address)}
        disabled={undo.isPending}
        className="self-start rounded-full border border-border-menu px-4 py-1.5 text-xs font-semibold text-ink-secondary hover:bg-bg-hover hover:text-ink-primary disabled:cursor-not-allowed disabled:opacity-60 sm:self-auto"
      >
        {undo.isPending ? 'Undoing…' : 'Undo'}
      </button>
    </div>
  );
}

function DeniedSendersList({
  senders,
  client,
}: {
  senders: DeniedSender[];
  client?: HailApiClient;
}) {
  return (
    <div className="space-y-3">
      <ListView
        items={senders}
        renderItem={(sender) => <DeniedSenderRow sender={sender} client={client} />}
        keyExtractor={(sender) => sender.sender_address}
        hasMore={false}
        isLoadingMore={false}
        onLoadMore={() => {}}
        emptyState={<p className="text-sm text-ink-tertiary">No denied senders yet.</p>}
      />
    </div>
  );
}

function PreviouslyDeniedSection({ client }: { client?: HailApiClient }) {
  const [expanded, setExpanded] = useState(false);
  const query = useDeniedSenders(client, { enabled: expanded });
  const deniedCount = query.data?.denied.length ?? 0;

  return (
    <section className="mt-8 rounded-lg border border-border-subtle bg-bg-surface/60 p-4">
      <button
        type="button"
        onClick={() => setExpanded((value) => !value)}
        aria-expanded={expanded}
        className="flex w-full items-center justify-between gap-4 text-left"
      >
        <span>
          <span className="block text-sm font-semibold text-ink-primary">
            Previously denied
          </span>
          <span className="mt-1 block text-xs text-ink-tertiary">
            Review blocked senders and undo mistakes.
          </span>
        </span>
        <span className="rounded-full border border-border-menu px-3 py-1 text-xs font-semibold text-ink-secondary">
          {expanded ? 'Hide' : 'Show'}
          {expanded && deniedCount > 0 ? ` (${deniedCount})` : ''}
        </span>
      </button>

      {expanded ? (
        <div className="mt-4">
          {query.isPending ? (
            <LoadingState label="Loading denied senders" />
          ) : query.isError ? (
            <ErrorState
              message={errorMessage(query.error)}
              onRetry={() => void query.refetch()}
            />
          ) : query.data.denied.length === 0 ? (
            <p className="text-sm text-ink-tertiary">No denied senders yet.</p>
          ) : (
            <DeniedSendersList senders={query.data.denied} client={client} />
          )}
        </div>
      ) : null}
    </section>
  );
}

export function ScreenerPage({ client }: { client?: HailApiClient } = {}) {
  const query = useScreenerView(client);
  const pendingCount = query.data?.senders.length ?? 0;

  let list;
  if (query.isPending) {
    list = <LoadingState label="Loading pending senders" />;
  } else if (query.isError) {
    list = (
      <ErrorState
        message={errorMessage(query.error)}
        onRetry={() => void query.refetch()}
      />
    );
  } else {
    list = (
      <div className="space-y-5">
        <ListView
          items={query.data.senders}
          renderItem={(sender) => <PendingSenderCard sender={sender} client={client} />}
          keyExtractor={(sender) => sender.sender}
          hasMore={false}
          isLoadingMore={false}
          onLoadMore={() => {}}
          emptyState={<EmptyState />}
        />
      </div>
    );
  }

  const pendingList = list;

  return (
    <AppShell
      title="The Screener"
      description="New senders end up here. Decide if they get in."
      actions={<ScreenerBanner pendingCount={pendingCount} />}
      list={
        <>
          {pendingList}
          <PreviouslyDeniedSection client={client} />
        </>
      }
    />
  );
}
