import { useId, useState, type FormEvent } from 'react';
import {
  HailApiError,
  type ScreenerClassification,
  type ScreenerPendingSender,
} from '../api/client';
import { useScreenerDecisionMutation, useScreenerView } from '../api/query';
import { useUndoToast } from '../components/UndoToastProvider';
import { AppShell } from '../layout/AppShell';

const classificationOptions: Array<{
  value: ScreenerClassification;
  label: string;
}> = [
  { value: 'imbox', label: 'Imbox' },
  { value: 'feed', label: 'Feed' },
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

function formatDate(value: string | null | undefined) {
  if (!value) {
    return 'Unknown';
  }

  const date = new Date(value);
  if (Number.isNaN(date.valueOf())) {
    return value;
  }

  return new Intl.DateTimeFormat(undefined, {
    month: 'short',
    day: 'numeric',
    year: date.getFullYear() === new Date().getFullYear() ? undefined : 'numeric',
    hour: 'numeric',
    minute: '2-digit',
  }).format(date);
}

function previewText(preview: unknown) {
  if (typeof preview === 'string') {
    return preview;
  }

  if (preview && typeof preview === 'object') {
    const record = preview as Record<string, unknown>;
    for (const key of ['text', 'body', 'preview', 'snippet', 'subject']) {
      const value = record[key];
      if (typeof value === 'string' && value.trim().length > 0) {
        return value;
      }
    }
  }

  return null;
}

function SkeletonList() {
  return (
    <div className="space-y-3" aria-label="Loading pending senders">
      {Array.from({ length: 4 }, (_, index) => (
        <div
          key={index}
          className="animate-pulse rounded-2xl border border-slate-800 bg-slate-900/60 p-4"
        >
          <div className="h-4 w-2/3 rounded bg-slate-800" />
          <div className="mt-3 h-3 w-1/2 rounded bg-slate-800" />
          <div className="mt-4 h-9 w-full rounded bg-slate-800" />
        </div>
      ))}
    </div>
  );
}

function StateCard({ title, body }: { title: string; body: string }) {
  return (
    <div className="flex min-h-64 flex-col items-center justify-center rounded-2xl border border-dashed border-slate-800 bg-slate-900/40 p-8 text-center">
      <p className="text-base font-semibold text-slate-200">{title}</p>
      <p className="mt-2 max-w-sm text-sm text-slate-400">{body}</p>
    </div>
  );
}

function PendingSenderCard({ sender }: { sender: ScreenerPendingSender }) {
  const selectId = useId();
  const [classifyAs, setClassifyAs] =
    useState<ScreenerClassification>('imbox');
  const { showToast } = useUndoToast();
  const decision = useScreenerDecisionMutation(undefined, {
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
  const preview = previewText(sender.latest_preview);

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
    <article className="rounded-2xl border border-slate-800 bg-slate-900/70 p-4 shadow-sm shadow-slate-950/40">
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0">
          <h2 className="truncate text-base font-semibold text-slate-100">
            {sender.sender || 'Unknown sender'}
          </h2>
          <p className="mt-1 text-sm text-slate-400">
            First seen <time>{formatDate(sender.first_seen_at)}</time>
          </p>
        </div>
        <span className="shrink-0 rounded-full border border-slate-700 bg-slate-950 px-2 py-1 text-xs font-semibold text-slate-300">
          {sender.message_count} {sender.message_count === 1 ? 'message' : 'messages'}
        </span>
      </div>

      {preview ? (
        <p className="mt-3 line-clamp-3 text-sm leading-6 text-slate-300">
          {preview}
        </p>
      ) : null}

      <form onSubmit={approve} className="mt-4 space-y-3">
        <label
          htmlFor={selectId}
          className="block text-sm font-medium text-slate-200"
        >
          Approve into
          <select
            id={selectId}
            value={classifyAs}
            onChange={(event) =>
              setClassifyAs(event.target.value as ScreenerClassification)
            }
            disabled={isPending}
            className="mt-2 w-full rounded-lg border border-slate-700 bg-slate-950 px-3 py-2 text-slate-100 outline-none ring-sky-400 transition focus:border-sky-400 focus:ring-2 disabled:cursor-not-allowed disabled:opacity-60"
          >
            {classificationOptions.map((option) => (
              <option key={option.value} value={option.value}>
                {option.label}
              </option>
            ))}
          </select>
        </label>

        <div className="grid grid-cols-2 gap-2">
          <button
            type="submit"
            disabled={isPending}
            className="rounded-lg bg-sky-400 px-3 py-2 text-sm font-semibold text-slate-950 transition hover:bg-sky-300 disabled:cursor-not-allowed disabled:opacity-60"
          >
            {isPending ? 'Saving…' : 'Approve'}
          </button>
          <button
            type="button"
            onClick={deny}
            disabled={isPending}
            className="rounded-lg border border-slate-700 px-3 py-2 text-sm font-semibold text-slate-100 transition hover:border-red-400 hover:text-red-100 disabled:cursor-not-allowed disabled:opacity-60"
          >
            Deny
          </button>
        </div>
      </form>

      {decision.isError ? (
        <p role="alert" className="mt-3 text-sm text-red-200">
          {decisionErrorMessage(decision.error)}
        </p>
      ) : null}
    </article>
  );
}

export function ScreenerPage() {
  const query = useScreenerView();

  let list;
  if (query.isPending) {
    list = <SkeletonList />;
  } else if (query.isError) {
    list = (
      <StateCard
        title="Could not load the Screener"
        body={errorMessage(query.error)}
      />
    );
  } else if (query.data.senders.length === 0) {
    list = (
      <StateCard
        title="No unknown senders"
        body="When mail arrives from a new sender, approve or deny it here before it reaches your regular views."
      />
    );
  } else {
    list = (
      <div className="space-y-3">
        {query.data.senders.map((sender) => (
          <PendingSenderCard key={sender.sender} sender={sender} />
        ))}
      </div>
    );
  }

  return (
    <AppShell
      title="Screener"
      description="Unknown senders wait here for approve or deny decisions."
      list={list}
      reading={
        <div className="rounded-2xl border border-slate-800 bg-slate-900/50 p-6">
          <h2 className="text-lg font-semibold text-slate-100">
            Screen once, route future mail
          </h2>
          <p className="mt-3 text-sm leading-6 text-slate-400">
            Approving a sender lets future messages land in Imbox, Feed, or
            Paper Trail. Denying keeps the sender out of your mail flow.
          </p>
        </div>
      }
    />
  );
}
