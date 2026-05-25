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
      <div>
        <h2 className="hail-sender truncate text-ink-primary">
          {senderIdentity.name}
        </h2>
        <p className="mt-1 truncate text-sm text-ink-secondary">
          {senderIdentity.email}
        </p>
      </div>

      <div className="mt-5 space-y-2">
        <p className="text-[0.95rem] leading-6 text-ink-secondary">{subject}</p>
        <p className="line-clamp-2 text-sm leading-6 text-ink-tertiary">
          {preview}
        </p>
      </div>

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
    <li className="flex flex-col gap-3 rounded-lg border border-border-subtle bg-bg-surface px-4 py-3 sm:flex-row sm:items-center sm:justify-between">
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
    </li>
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
            <ul className="space-y-2">
              {query.data.denied.map((sender) => (
                <DeniedSenderRow
                  key={sender.sender_address}
                  sender={sender}
                  client={client}
                />
              ))}
            </ul>
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
  } else if (query.data.senders.length === 0) {
    list = <EmptyState />;
  } else {
    list = (
      <div className="space-y-5">
        {query.data.senders.map((sender) => (
          <PendingSenderCard key={sender.sender} sender={sender} client={client} />
        ))}
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
