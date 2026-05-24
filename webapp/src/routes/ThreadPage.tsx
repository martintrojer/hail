import { useNavigate } from '@tanstack/react-router';
import { useState, type MouseEvent } from 'react';
import {
  HailApiError,
  type MailClassification,
  type ThreadMessage,
  type ThreadViewResponse,
} from '../api/client';
import {
  useArchiveThreadMutation,
  useBubbleUpMutation,
  useClassifyThreadMutation,
  useReplyLaterThreadMutation,
  useSetAsideThreadMutation,
  useThread,
  useTrashThreadMutation,
} from '../api/query';
import { ContactNotePanel } from '../components/ContactNotePanel';
import { icons, iconSizeProps } from '../components/icons';
import { MessageActionPopup } from '../components/MessageActionPopup';
import { useUndoToast } from '../components/UndoToastProvider';
import { AppShell } from '../layout/AppShell';

const classificationOptions: Array<{
  value: MailClassification;
  label: string;
}> = [
  { value: 'imbox', label: 'Imbox' },
  { value: 'feed', label: 'Feed' },
  { value: 'papertrail', label: 'Paper Trail' },
];

interface ThreadPageProps {
  threadId: string;
  client?: Parameters<typeof useThread>[1];
}

function formatParticipantName(participant: { name?: string | null; email: string }) {
  return participant.name?.trim() || participant.email || 'Unknown';
}

function formatParticipantList(
  participants: Array<{ name?: string | null; email: string }>,
) {
  if (participants.length === 0) {
    return 'Unknown';
  }

  return participants.map(formatParticipantName).join(', ');
}

function formatDate(value: string | null | undefined) {
  if (!value) {
    return 'No date';
  }

  const date = new Date(value);
  if (Number.isNaN(date.valueOf())) {
    return value;
  }

  return new Intl.DateTimeFormat(undefined, {
    dateStyle: 'medium',
    timeStyle: 'short',
  }).format(date);
}

function errorCopy(error: Error) {
  if (error instanceof HailApiError) {
    if (error.status === 401) {
      return 'Your session expired. Sign in again to open this thread.';
    }
    if (error.status === 404) {
      return 'This thread was not found. It may have moved or been deleted.';
    }
    if (error.status === 400) {
      return 'This thread link is invalid.';
    }
    return `Thread failed with HTTP ${error.status}.`;
  }

  return 'Thread failed to load. Refresh and try again.';
}

function threadActionErrorMessage(error: Error) {
  if (error instanceof HailApiError) {
    if (error.status === 401) {
      return 'Your session expired. Sign in again before changing this thread.';
    }
    if (error.status === 404) {
      return 'This thread was not found. Refresh and try again.';
    }
    return `Thread action failed with HTTP ${error.status}.`;
  }

  return 'Thread action failed. Try again.';
}

function StateCard({ title, body }: { title: string; body: string }) {
  return (
    <div className="flex min-h-80 flex-col items-center justify-center rounded-2xl border border-dashed border-slate-800 bg-slate-900/40 p-8 text-center lg:min-h-full">
      <p className="text-base font-semibold text-slate-200">{title}</p>
      <p className="mt-2 max-w-md text-sm text-slate-400">{body}</p>
    </div>
  );
}

function ThreadSkeleton() {
  return (
    <div className="space-y-4">
      <div className="rounded-3xl border border-slate-800 bg-slate-900/70 p-6">
        <div className="h-8 w-2/3 animate-pulse rounded bg-slate-800" />
        <div className="mt-4 h-4 w-1/2 animate-pulse rounded bg-slate-800" />
      </div>
      {Array.from({ length: 2 }, (_, index) => (
        <div
          key={index}
          className="rounded-3xl border border-slate-800 bg-slate-900/60 p-5"
        >
          <div className="h-4 w-1/3 animate-pulse rounded bg-slate-800" />
          <div className="mt-5 h-24 animate-pulse rounded bg-slate-800" />
        </div>
      ))}
    </div>
  );
}

function ThreadSummary({ thread }: { thread: ThreadViewResponse }) {
  return (
    <div className="rounded-3xl border border-slate-800 bg-slate-900/70 p-6 shadow-xl shadow-slate-950/30">
      <p className="text-xs font-semibold uppercase tracking-[0.3em] text-sky-300">
        Thread
      </p>
      <h2 className="mt-3 text-3xl font-semibold tracking-tight text-slate-50">
        {thread.subject || '(no subject)'}
      </h2>
      <p className="mt-3 text-sm leading-6 text-slate-400">
        {thread.messages.length}{' '}
        {thread.messages.length === 1 ? 'message' : 'messages'} with{' '}
        {formatParticipantList(thread.participants)}
      </p>
    </div>
  );
}

function TrackerBadge({ message }: { message: ThreadMessage }) {
  const count = message.blocked_trackers.length;
  if (count === 0) {
    return null;
  }

  return (
    <span
      className="rounded-full border border-amber-400/40 bg-amber-400/10 px-2.5 py-1 text-xs font-semibold text-amber-100"
      title={message.blocked_trackers.map((tracker) => tracker.reason).join('\n')}
    >
      {count} tracker{count === 1 ? '' : 's'} blocked
    </span>
  );
}

function MessageCard({ message }: { message: ThreadMessage }) {
  const [popupAnchor, setPopupAnchor] = useState<DOMRect | null>(null);
  const popupOpen = popupAnchor !== null;
  const MoreIcon = icons.more;

  function openPopup(event: MouseEvent<HTMLButtonElement>) {
    setPopupAnchor(event.currentTarget.getBoundingClientRect());
  }

  return (
    <article className="rounded-3xl border border-slate-800 bg-slate-900/60 p-5 shadow-lg shadow-slate-950/20">
      <header className="flex flex-col gap-3 border-b border-slate-800 pb-4 sm:flex-row sm:items-start sm:justify-between">
        <div className="min-w-0">
          <p className="truncate text-sm font-semibold text-slate-100">
            {formatParticipantList(message.from)}
          </p>
          <p className="mt-1 truncate text-xs text-slate-500">
            To {formatParticipantList(message.to)}
          </p>
        </div>
        <div className="flex shrink-0 flex-wrap items-center gap-2 sm:justify-end">
          <TrackerBadge message={message} />
          <time className="rounded-full bg-slate-950 px-2.5 py-1 text-xs text-slate-400">
            {formatDate(message.received_at)}
          </time>
          <button
            type="button"
            aria-label="Message actions"
            aria-haspopup="menu"
            aria-expanded={popupOpen}
            onClick={openPopup}
            className="rounded-full p-1.5 text-slate-400 transition hover:bg-slate-800 hover:text-slate-100 focus:bg-slate-800 focus:text-slate-100 focus:outline-none focus:ring-2 focus:ring-sky-400"
          >
            <MoreIcon {...iconSizeProps.sm} aria-hidden="true" />
          </button>
          <MessageActionPopup
            open={popupOpen}
            anchorRect={popupAnchor}
            onClose={() => setPopupAnchor(null)}
            onAction={() => setPopupAnchor(null)}
          />
        </div>
      </header>

      {message.html.trim().length > 0 ? (
        <div
          className="mt-5 max-w-none overflow-x-auto text-sm leading-7 text-slate-200 [&_a]:text-sky-300 [&_a]:underline [&_blockquote]:border-l-2 [&_blockquote]:border-slate-700 [&_blockquote]:pl-4 [&_blockquote]:text-slate-400 [&_code]:rounded [&_code]:bg-slate-950 [&_code]:px-1 [&_img]:max-w-full [&_p]:my-3 [&_table]:w-full [&_table]:border-collapse [&_td]:border [&_td]:border-slate-800 [&_td]:p-2 [&_th]:border [&_th]:border-slate-800 [&_th]:p-2"
          // Server owns the mail-render trust boundary: hail-api strips quoted
          // history, removes trackers, and sanitizes HTML before this field is
          // exposed to the SPA. The client renders only that sanitized fragment.
          dangerouslySetInnerHTML={{ __html: message.html }}
        />
      ) : (
        <p className="mt-5 whitespace-pre-wrap text-sm leading-7 text-slate-300">
          {message.preview || 'This message has no renderable body.'}
        </p>
      )}
    </article>
  );
}

function primarySender(thread: ThreadViewResponse) {
  for (const message of thread.messages) {
    const sender = message.from.find(
      (participant) => participant.email.trim().length > 0,
    );
    if (sender) {
      return sender;
    }
  }

  return (
    thread.participants.find(
      (participant) => participant.email.trim().length > 0,
    ) ?? null
  );
}

function tomorrowAt(hour: number, minute = 0) {
  const date = new Date();
  date.setDate(date.getDate() + 1);
  date.setHours(hour, minute, 0, 0);
  return date;
}

function datetimeLocalValue(date: Date) {
  const year = date.getFullYear();
  const month = String(date.getMonth() + 1).padStart(2, '0');
  const day = String(date.getDate()).padStart(2, '0');
  const hours = String(date.getHours()).padStart(2, '0');
  const minutes = String(date.getMinutes()).padStart(2, '0');
  return `${year}-${month}-${day}T${hours}:${minutes}`;
}

function toApiDateTime(localValue: string) {
  return new Date(localValue).toISOString();
}

function ThreadActions({
  thread,
  client,
}: {
  thread: ThreadViewResponse;
  client?: Parameters<typeof useThread>[1];
}) {
  const { showToast } = useUndoToast();
  const [classification, setClassification] =
    useState<MailClassification>('imbox');
  const classify = useClassifyThreadMutation(client, {
    onSuccess: (data, variables) => {
      const label =
        classificationOptions.find((option) => option.value === variables.to)
          ?.label ?? variables.to;
      showToast({
        message: `Moved thread to ${label}.`,
        undo: data.undo ? { id: data.undo.id } : null,
        undoSuccessMessage: 'Thread classification undone.',
      });
    },
  });
  const archive = useArchiveThreadMutation(client, {
    onSuccess: (data) => {
      showToast({
        message: 'Thread archived.',
        undo: data.undo ? { id: data.undo.id } : null,
        undoSuccessMessage: 'Archive undone.',
      });
    },
  });
  const trash = useTrashThreadMutation(client, {
    onSuccess: (data) => {
      showToast({
        message: 'Thread moved to trash.',
        undo: data.undo ? { id: data.undo.id } : null,
        undoSuccessMessage: 'Trash undone.',
      });
    },
  });
  const setAside = useSetAsideThreadMutation(client, {
    onSuccess: (data) => {
      showToast({
        message: 'Thread added to Set Aside.',
        undo: data.undo ? { id: data.undo.id } : null,
        undoSuccessMessage: 'Set Aside undone.',
      });
    },
  });
  const replyLater = useReplyLaterThreadMutation(client, {
    onSuccess: (data) => {
      showToast({
        message: 'Thread added to Reply Later.',
        undo: data.undo ? { id: data.undo.id } : null,
        undoSuccessMessage: 'Reply Later undone.',
      });
    },
  });
  const [bubbleAt, setBubbleAt] = useState(() => datetimeLocalValue(tomorrowAt(9)));
  const bubbleUp = useBubbleUpMutation(client, {
    onSuccess: (data) => {
      showToast({
        message: `Thread will bubble up ${formatDate(data.surface_at)}.`,
      });
    },
  });
  const busy =
    classify.isPending ||
    archive.isPending ||
    trash.isPending ||
    setAside.isPending ||
    replyLater.isPending ||
    bubbleUp.isPending;
  const error =
    classify.error ??
    archive.error ??
    trash.error ??
    setAside.error ??
    replyLater.error ??
    bubbleUp.error;

  function scheduleBubbleUp() {
    if (!bubbleAt) {
      return;
    }

    bubbleUp.mutate({
      threadId: thread.thread_id,
      request: { at: toApiDateTime(bubbleAt) },
    });
  }

  return (
    <div className="rounded-3xl border border-slate-800 bg-slate-900/70 p-4 shadow-lg shadow-slate-950/20">
      <div className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
        <label className="min-w-0 flex-1 text-sm font-medium text-slate-200">
          Classify as
          <select
            value={classification}
            onChange={(event) =>
              setClassification(event.target.value as MailClassification)
            }
            disabled={busy}
            className="mt-2 w-full rounded-lg border border-slate-700 bg-slate-950 px-3 py-2 text-slate-100 outline-none ring-sky-400 transition focus:border-sky-400 focus:ring-2 disabled:cursor-not-allowed disabled:opacity-60"
          >
            {classificationOptions.map((option) => (
              <option key={option.value} value={option.value}>
                {option.label}
              </option>
            ))}
          </select>
        </label>

        <div className="flex flex-wrap gap-2 sm:self-end">
          <button
            type="button"
            disabled={busy}
            onClick={() =>
              classify.mutate({ threadId: thread.thread_id, to: classification })
            }
            className="rounded-lg border border-slate-700 px-3 py-2 text-sm font-semibold text-slate-100 transition hover:border-sky-400 hover:text-sky-200 disabled:cursor-not-allowed disabled:opacity-60"
          >
            {classify.isPending ? 'Moving…' : 'Move'}
          </button>
          <button
            type="button"
            disabled={busy}
            onClick={() => archive.mutate({ threadId: thread.thread_id })}
            className="rounded-lg border border-slate-700 px-3 py-2 text-sm font-semibold text-slate-100 transition hover:border-emerald-400 hover:text-emerald-100 disabled:cursor-not-allowed disabled:opacity-60"
          >
            {archive.isPending ? 'Archiving…' : 'Archive'}
          </button>
          <button
            type="button"
            disabled={busy}
            onClick={() => trash.mutate({ threadId: thread.thread_id })}
            className="rounded-lg border border-slate-700 px-3 py-2 text-sm font-semibold text-slate-100 transition hover:border-red-400 hover:text-red-100 disabled:cursor-not-allowed disabled:opacity-60"
          >
            {trash.isPending ? 'Trashing…' : 'Trash'}
          </button>
        </div>
      </div>

      <div className="mt-4 border-t border-slate-800 pt-4">
        <p className="text-xs font-semibold uppercase tracking-[0.25em] text-slate-500">
          Stack and bubble
        </p>
        <div className="mt-3 flex flex-wrap gap-2">
          <button
            type="button"
            disabled={busy}
            onClick={() => setAside.mutate({ threadId: thread.thread_id })}
            className="rounded-lg border border-slate-700 px-3 py-2 text-sm font-semibold text-slate-100 transition hover:border-violet-400 hover:text-violet-100 disabled:cursor-not-allowed disabled:opacity-60"
          >
            {setAside.isPending ? 'Setting aside…' : 'Set Aside'}
          </button>
          <button
            type="button"
            disabled={busy}
            onClick={() => replyLater.mutate({ threadId: thread.thread_id })}
            className="rounded-lg border border-slate-700 px-3 py-2 text-sm font-semibold text-slate-100 transition hover:border-amber-400 hover:text-amber-100 disabled:cursor-not-allowed disabled:opacity-60"
          >
            {replyLater.isPending ? 'Saving…' : 'Reply Later'}
          </button>
        </div>
        <div className="mt-3 flex flex-col gap-2 sm:flex-row sm:items-end">
          <label className="min-w-0 flex-1 text-sm font-medium text-slate-200">
            Bubble up at
            <input
              type="datetime-local"
              value={bubbleAt}
              onChange={(event) => setBubbleAt(event.target.value)}
              disabled={busy}
              className="mt-2 w-full rounded-lg border border-slate-700 bg-slate-950 px-3 py-2 text-slate-100 outline-none ring-sky-400 transition focus:border-sky-400 focus:ring-2 disabled:cursor-not-allowed disabled:opacity-60"
            />
          </label>
          <button
            type="button"
            disabled={busy || !bubbleAt}
            onClick={scheduleBubbleUp}
            className="rounded-lg border border-slate-700 px-3 py-2 text-sm font-semibold text-slate-100 transition hover:border-sky-400 hover:text-sky-100 disabled:cursor-not-allowed disabled:opacity-60"
          >
            {bubbleUp.isPending ? 'Scheduling…' : 'Bubble Up'}
          </button>
        </div>
      </div>

      {error ? (
        <p role="alert" className="mt-3 text-sm text-red-200">
          {threadActionErrorMessage(error)}
        </p>
      ) : null}
    </div>
  );
}

function ThreadDocument({
  thread,
  client,
}: {
  thread: ThreadViewResponse;
  client?: Parameters<typeof useThread>[1];
}) {
  const navigate = useNavigate();
  const sender = primarySender(thread);

  function reply() {
    void navigate({ to: '/thread/$threadId/reply', params: { threadId: thread.thread_id } });
  }

  const replyButton = (
    <button
      type="button"
      onClick={reply}
      className="rounded-lg bg-sky-400 px-4 py-2 text-sm font-semibold text-slate-950 transition hover:bg-sky-300"
    >
      Reply
    </button>
  );

  if (thread.messages.length === 0) {
    return (
      <div className="space-y-4">
        <ThreadSummary thread={thread} />
        <ThreadActions thread={thread} client={client} />
        {replyButton}
        {sender ? (
          <ContactNotePanel address={sender.email} displayName={sender.name} />
        ) : null}
        <StateCard
          title="No messages in this thread"
          body="The server returned the thread but did not include any messages."
        />
      </div>
    );
  }

  return (
    <div className="space-y-4">
      <ThreadSummary thread={thread} />
      <ThreadActions thread={thread} client={client} />
      {replyButton}
      {sender ? (
        <ContactNotePanel address={sender.email} displayName={sender.name} />
      ) : null}
      <div className="space-y-4">
        {thread.messages.map((message) => (
          <MessageCard key={message.email_id} message={message} />
        ))}
      </div>
    </div>
  );
}

export function ThreadPage({ threadId, client }: ThreadPageProps) {
  const query = useThread(threadId, client);

  let reading;
  if (query.isPending) {
    reading = <ThreadSkeleton />;
  } else if (query.isError) {
    reading = (
      <StateCard
        title={
          query.error instanceof HailApiError && query.error.status === 404
            ? 'Thread not found'
            : 'Could not load thread'
        }
        body={errorCopy(query.error)}
      />
    );
  } else {
    reading = <ThreadDocument thread={query.data} client={client} />;
  }

  return (
    <AppShell
      title="Thread"
      description={query.data?.subject || 'Reading pane'}
      reading={reading}
    />
  );
}
