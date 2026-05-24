import { useId, useState, type FormEvent } from 'react';
import type { HailApiClient } from '../api/client';
import {
  HailApiError,
  type ScreenerClassification,
  type ScreenerPendingSender,
} from '../api/client';
import { useScreenerDecisionMutation, useScreenerView } from '../api/query';
import { ScreenerBanner } from '../components/ScreenerBanner';
import { useUndoToast } from '../components/UndoToastProvider';
import { AppShell } from '../layout/AppShell';

const classificationOptions: Array<{
  value: ScreenerClassification;
  label: string;
}> = [
  { value: 'imbox', label: 'Imbox' },
  { value: 'feed', label: 'The Feed' },
  { value: 'papertrail', label: 'Paper Trail' },
];

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

function SkeletonList() {
  return (
    <div className="space-y-5" aria-label="Loading pending senders">
      {Array.from({ length: 3 }, (_, index) => (
        <div key={index} className="animate-pulse rounded-lg bg-bg-surface p-5">
          <div className="h-5 w-2/5 rounded bg-bg-selected" />
          <div className="mt-2 h-4 w-1/2 rounded bg-bg-selected" />
          <div className="mt-5 h-4 w-3/4 rounded bg-bg-hover" />
          <div className="mt-3 h-4 w-full rounded bg-bg-hover" />
          <div className="mt-5 flex gap-3">
            <div className="h-10 w-28 rounded-lg bg-bg-selected" />
            <div className="h-10 w-24 rounded-lg bg-bg-hover" />
          </div>
        </div>
      ))}
    </div>
  );
}

function ErrorState({ body }: { body: string }) {
  return (
    <div className="flex min-h-64 flex-col items-center justify-center rounded-lg bg-bg-surface p-8 text-center">
      <p className="text-base font-semibold text-ink-primary">
        Could not load the Screener
      </p>
      <p className="mt-2 max-w-sm text-sm leading-6 text-ink-secondary">{body}</p>
    </div>
  );
}

function EmptyState() {
  return (
    <div className="flex min-h-64 items-center justify-center text-center">
      <p className="hail-body text-ink-secondary">
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
  const selectId = useId();
  const [classifyAs, setClassifyAs] =
    useState<ScreenerClassification>('imbox');
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

  function approve(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    decision.mutate({
      sender: sender.sender,
      decision: 'approve',
      classify_as: classifyAs,
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

      <form onSubmit={approve} className="mt-5 space-y-4">
        <label
          htmlFor={selectId}
          className="block text-sm font-medium text-ink-secondary"
        >
          Approve into
          <select
            id={selectId}
            value={classifyAs}
            onChange={(event) =>
              setClassifyAs(event.target.value as ScreenerClassification)
            }
            disabled={isPending}
            className="mt-2 w-full rounded-lg border border-border-menu bg-bg-page px-3 py-2 text-sm text-ink-primary outline-none focus:border-accent-blue focus:ring-2 focus:ring-accent-blue/25 disabled:cursor-not-allowed disabled:opacity-60"
          >
            {classificationOptions.map((option) => (
              <option key={option.value} value={option.value}>
                {option.label}
              </option>
            ))}
          </select>
        </label>

        <div className="flex flex-wrap gap-3">
          <button
            type="submit"
            aria-label={isPending ? 'Saving…' : 'Approve'}
            disabled={isPending}
            className="rounded-lg bg-accent-blue px-4 py-2 text-sm font-semibold text-white hover:bg-accent-blue-hover disabled:cursor-not-allowed disabled:opacity-60"
          >
            {isPending ? 'Saving…' : 'Yes'}
          </button>
          <button
            type="button"
            aria-label="Deny"
            onClick={deny}
            disabled={isPending}
            className="rounded-lg border border-border-menu px-4 py-2 text-sm font-semibold text-ink-secondary hover:bg-bg-hover hover:text-ink-primary disabled:cursor-not-allowed disabled:opacity-60"
          >
            No
          </button>
        </div>
      </form>

      {decision.isError ? (
        <p role="alert" className="mt-4 text-sm text-accent-red">
          {decisionErrorMessage(decision.error)}
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
    list = <SkeletonList />;
  } else if (query.isError) {
    list = <ErrorState body={errorMessage(query.error)} />;
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

  return (
    <AppShell
      title="The Screener"
      description="New senders end up here. Decide if they get in."
      actions={<ScreenerBanner pendingCount={pendingCount} />}
      list={list}
    />
  );
}
