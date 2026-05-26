import { useRef, useState } from 'react';
import { Link } from '@tanstack/react-router';
import { ShieldOff } from 'lucide-react';
import type { HailApiClient } from '../api/client';
import {
  type ScreenerClassification,
  type ScreenerPendingSender,
} from '../api/client';
import {
  useScreenerDecisionMutation,
  useScreenerView,
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
import { pillButtonClass } from '../lib/buttonStyles';
import { formatDate } from '../lib/dates';
import { actionErrorMessage, viewErrorMessage } from '../lib/errorMessages';

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
  const senderIdentity = {
    name: sender.sender || 'Unknown sender',
    email: sender.sender || 'unknown address',
  };
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
                      {formatDate(email.received_at)}
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
          className={pillButtonClass('primary', 'md')}
        >
          {isPending ? 'Saving…' : 'Yes'}
        </button>
        <button
          type="button"
          aria-label="Deny"
          onClick={deny}
          disabled={isPending}
          className={pillButtonClass('outline', 'md')}
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
          {actionErrorMessage(decision.error, 'Decision')}
        </p>
      ) : null}
    </article>
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
        message={viewErrorMessage(query.error, 'Screener')}
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
      actions={
        <div className="flex items-center gap-4">
          <ScreenerBanner pendingCount={pendingCount} />
          <Link
            to="/screened-out"
            className="inline-flex items-center gap-1.5 text-sm font-medium text-ink-secondary hover:text-ink-primary"
          >
            <ShieldOff size={14} />
            Screened Out
          </Link>
        </div>
      }
      list={pendingList}
    />
  );
}
