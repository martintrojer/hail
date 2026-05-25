import { useNavigate } from '@tanstack/react-router';
import { useMemo, useRef, useState, type MouseEvent } from 'react';
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
import { AddNoteForm } from '../components/AddNoteForm';
import { ErrorState } from '../components/ErrorState';
import { InlineNote, type InlineNoteProps } from '../components/InlineNote';
import {
  ArrowLeft,
  Clock,
  MoreHorizontal,
  Paperclip,
  iconSizeProps,
} from '../components/icons';
import { LoadingState } from '../components/LoadingState';
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

interface LocalNote extends InlineNoteProps {
  id: string;
  messageId: string;
}

function formatParticipantName(participant: {
  name?: string | null;
  email: string;
}) {
  return participant.name?.trim() || participant.email || 'Unknown';
}

function formatParticipantEmail(participant: { email: string } | null) {
  return participant?.email.trim() || 'unknown sender';
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
    <div className="flex min-h-[300px] flex-col items-center justify-center p-8 text-center">
      <p className="text-lg font-semibold text-ink-primary">{title}</p>
      <p className="mt-2 max-w-md text-sm leading-6 text-ink-secondary">{body}</p>
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
      className="rounded-full bg-bg-banner px-2 py-0.5 text-xs font-semibold text-ink-secondary"
      title={message.blocked_trackers
        .map((tracker) => tracker.reason)
        .join('\n')}
    >
      {count} tracker{count === 1 ? '' : 's'} blocked
    </span>
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

function firstSender(message: ThreadMessage) {
  return (
    message.from.find((participant) => participant.email.trim().length > 0) ??
    message.from[0] ??
    null
  );
}

function participantInitial(
  participant: { name?: string | null; email: string } | null,
) {
  const source = participant ? formatParticipantName(participant) : 'Unknown';
  return source.trim().charAt(0).toUpperCase() || 'U';
}

function sortedMessages(messages: ThreadMessage[]) {
  return [...messages].sort((left, right) => {
    const leftTime = left.received_at
      ? new Date(left.received_at).valueOf()
      : Number.MAX_SAFE_INTEGER;
    const rightTime = right.received_at
      ? new Date(right.received_at).valueOf()
      : Number.MAX_SAFE_INTEGER;

    if (Number.isNaN(leftTime) && Number.isNaN(rightTime)) {
      return 0;
    }
    if (Number.isNaN(leftTime)) {
      return 1;
    }
    if (Number.isNaN(rightTime)) {
      return -1;
    }

    return leftTime - rightTime;
  });
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
  const navigate = useNavigate();
  const { showToast } = useUndoToast();
  const [classification, setClassification] =
    useState<MailClassification>('imbox');
  function returnToPreviousList() {
    if (window.history.length > 1) {
      window.history.back();
      return;
    }

    void navigate({ to: '/imbox' });
  }

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
      returnToPreviousList();
    },
  });
  const archive = useArchiveThreadMutation(client, {
    onSuccess: (data) => {
      showToast({
        message: 'Thread archived.',
        undo: data.undo ? { id: data.undo.id } : null,
        undoSuccessMessage: 'Archive undone.',
      });
      returnToPreviousList();
    },
  });
  const trash = useTrashThreadMutation(client, {
    onSuccess: (data) => {
      showToast({
        message: 'Thread moved to trash.',
        undo: data.undo ? { id: data.undo.id } : null,
        undoSuccessMessage: 'Trash undone.',
      });
      returnToPreviousList();
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
  const [bubbleAt, setBubbleAt] = useState(() =>
    datetimeLocalValue(tomorrowAt(9)),
  );
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
    <section
      className="rounded-lg border border-border-hairline bg-bg-surface p-4"
      aria-label="Thread actions"
    >
      <div className="flex flex-col gap-3 sm:flex-row sm:items-end">
        <label className="min-w-0 flex-1 text-sm font-medium text-ink-secondary">
          Move to
          <select
            value={classification}
            onChange={(event) =>
              setClassification(event.target.value as MailClassification)
            }
            disabled={busy}
            className="mt-2 w-full rounded-lg border border-border-hairline bg-bg-page px-3 py-2 text-ink-primary outline-none focus:border-accent-blue disabled:cursor-not-allowed disabled:opacity-60"
          >
            {classificationOptions.map((option) => (
              <option key={option.value} value={option.value}>
                {option.label}
              </option>
            ))}
          </select>
        </label>

        <div className="flex flex-wrap gap-2">
          <button
            type="button"
            disabled={busy}
            onClick={() =>
              classify.mutate({
                threadId: thread.thread_id,
                to: classification,
              })
            }
            className="rounded-lg border border-border-menu px-3 py-2 text-sm font-semibold text-ink-secondary hover:bg-bg-hover hover:text-ink-primary disabled:cursor-not-allowed disabled:opacity-60"
          >
            {classify.isPending ? 'Moving…' : 'Move'}
          </button>
          <button
            type="button"
            disabled={busy}
            onClick={() => archive.mutate({ threadId: thread.thread_id })}
            className="rounded-lg border border-border-menu px-3 py-2 text-sm font-semibold text-ink-secondary hover:bg-bg-hover hover:text-ink-primary disabled:cursor-not-allowed disabled:opacity-60"
          >
            {archive.isPending ? 'Archiving…' : 'Archive'}
          </button>
          <button
            type="button"
            disabled={busy}
            onClick={() => trash.mutate({ threadId: thread.thread_id })}
            className="rounded-lg border border-border-menu px-3 py-2 text-sm font-semibold text-accent-red hover:bg-bg-hover disabled:cursor-not-allowed disabled:opacity-60"
          >
            {trash.isPending ? 'Trashing…' : 'Trash'}
          </button>
        </div>
      </div>

      <div className="mt-4 flex flex-col gap-3 border-t border-border-hairline pt-4 sm:flex-row sm:items-end">
        <div className="flex flex-wrap gap-2">
          <button
            type="button"
            disabled={busy}
            onClick={() => setAside.mutate({ threadId: thread.thread_id })}
            className="rounded-lg border border-border-menu px-3 py-2 text-sm font-semibold text-ink-secondary hover:bg-bg-hover hover:text-ink-primary disabled:cursor-not-allowed disabled:opacity-60"
          >
            {setAside.isPending ? 'Setting aside…' : 'Set Aside'}
          </button>
          <button
            type="button"
            disabled={busy}
            onClick={() => replyLater.mutate({ threadId: thread.thread_id })}
            className="rounded-lg border border-border-menu px-3 py-2 text-sm font-semibold text-ink-secondary hover:bg-bg-hover hover:text-ink-primary disabled:cursor-not-allowed disabled:opacity-60"
          >
            {replyLater.isPending ? 'Saving…' : 'Reply Later'}
          </button>
        </div>

        <label className="min-w-0 flex-1 text-sm font-medium text-ink-secondary">
          Bubble up at
          <input
            type="datetime-local"
            value={bubbleAt}
            onChange={(event) => setBubbleAt(event.target.value)}
            disabled={busy}
            className="mt-2 w-full rounded-lg border border-border-hairline bg-bg-page px-3 py-2 text-ink-primary outline-none focus:border-accent-blue disabled:cursor-not-allowed disabled:opacity-60"
          />
        </label>
        <button
          type="button"
          disabled={busy || !bubbleAt}
          onClick={scheduleBubbleUp}
          className="rounded-lg border border-border-menu px-3 py-2 text-sm font-semibold text-ink-secondary hover:bg-bg-hover hover:text-ink-primary disabled:cursor-not-allowed disabled:opacity-60"
        >
          {bubbleUp.isPending ? 'Scheduling…' : 'Bubble Up'}
        </button>
      </div>

      {error ? (
        <p role="alert" className="mt-3 text-sm text-accent-red">
          {threadActionErrorMessage(error)}
        </p>
      ) : null}
    </section>
  );
}

function MessageCard({
  message,
  notes,
  addingNote,
  popupOpen,
  popupAnchor,
  onTogglePopup,
  onClosePopup,
  onStartAddNote,
  onCancelAddNote,
  onSaveNote,
}: {
  message: ThreadMessage;
  notes: LocalNote[];
  addingNote: boolean;
  popupOpen: boolean;
  popupAnchor: DOMRect | null;
  onTogglePopup: (messageId: string, anchorRect: DOMRect) => void;
  onClosePopup: () => void;
  onStartAddNote: (messageId: string) => void;
  onCancelAddNote: () => void;
  onSaveNote: (messageId: string, text: string) => void;
}) {
  const sender = firstSender(message);

  function togglePopup(event: MouseEvent<HTMLButtonElement>) {
    onTogglePopup(message.email_id, event.currentTarget.getBoundingClientRect());
  }

  function handlePopupAction(action: string) {
    if (action === 'add-note') {
      onStartAddNote(message.email_id);
    }
    onClosePopup();
  }

  return (
    <article className="border-b border-border-hairline pb-8 last:border-b-0">
      <header className="flex items-start gap-3">
        <div className="grid h-8 w-8 shrink-0 place-items-center rounded-full bg-bg-hover text-xs font-semibold text-ink-secondary">
          <span aria-hidden="true">{participantInitial(sender)}</span>
        </div>
        <div className="min-w-0 flex-1">
          <div className="flex items-start justify-between gap-3">
            <div className="min-w-0">
              <p className="truncate font-semibold text-ink-primary">
                {sender ? formatParticipantName(sender) : 'Unknown sender'}
              </p>
              <p className="mt-0.5 truncate text-sm text-ink-tertiary">
                To {formatParticipantList(message.to)} ·{' '}
                <time>{formatDate(message.received_at)}</time>
              </p>
            </div>
            <div className="flex shrink-0 items-center gap-2">
              <TrackerBadge message={message} />
              <button
                type="button"
                aria-label="Message actions"
                aria-haspopup="menu"
                aria-expanded={popupOpen}
                onMouseDown={(event) => event.stopPropagation()}
                onClick={togglePopup}
                className="rounded-full p-1 text-ink-tertiary outline-none hover:bg-hover hover:text-ink-primary focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent-blue"
              >
                <MoreHorizontal {...iconSizeProps.sm} aria-hidden="true" />
              </button>
              <MessageActionPopup
                open={popupOpen}
                anchorRect={popupAnchor}
                onClose={onClosePopup}
                onAction={handlePopupAction}
              />
            </div>
          </div>

          {message.html.trim().length > 0 ? (
            <div
              className="mt-5 max-w-none overflow-x-auto text-base leading-relaxed text-ink-primary [&_a]:text-accent-blue [&_a]:underline [&_blockquote]:border-l-2 [&_blockquote]:border-border-hairline [&_blockquote]:pl-4 [&_blockquote]:text-ink-secondary [&_code]:rounded [&_code]:bg-bg-hover [&_code]:px-1 [&_img]:max-w-full [&_p]:my-3 [&_table]:w-full [&_table]:border-collapse [&_td]:border [&_td]:border-border-hairline [&_td]:p-2 [&_th]:border [&_th]:border-border-hairline [&_th]:p-2"
              // Server owns the mail-render trust boundary: hail-api strips quoted
              // history, removes trackers, and sanitizes HTML before this field is
              // exposed to the SPA. The client renders only that sanitized fragment.
              dangerouslySetInnerHTML={{ __html: message.html }}
            />
          ) : (
            <p className="mt-5 whitespace-pre-wrap text-base leading-relaxed text-ink-primary">
              {message.preview || 'This message has no renderable body.'}
            </p>
          )}

          {notes.length > 0 ? (
            <div className="mt-5 space-y-3">
              {notes.map((note) => (
                <InlineNote
                  key={note.id}
                  text={note.text}
                  author={note.author}
                  timestamp={note.timestamp}
                />
              ))}
            </div>
          ) : null}

          {addingNote ? (
            <div className="mt-5 rounded-r-lg border-l-4 border-accent-yellow bg-bg-banner p-4">
              <AddNoteForm
                onSave={(text) => onSaveNote(message.email_id, text)}
                onCancel={onCancelAddNote}
              />
            </div>
          ) : null}
        </div>
      </header>
    </article>
  );
}

function MiniReplyComposer({ senderName }: { senderName: string }) {
  const textareaRef = useRef<HTMLTextAreaElement | null>(null);

  function resizeTextarea() {
    const textarea = textareaRef.current;
    if (!textarea) {
      return;
    }

    textarea.style.height = 'auto';
    textarea.style.height = `${textarea.scrollHeight}px`;
  }

  return (
    <section className="pt-2" aria-label="Reply composer">
      <textarea
        ref={textareaRef}
        rows={3}
        onInput={resizeTextarea}
        placeholder={`Reply to ${senderName}…`}
        className="min-h-28 w-full resize-none rounded-lg border border-border-hairline bg-bg-surface p-3 text-base leading-relaxed text-ink-primary outline-none placeholder:text-ink-tertiary focus:border-accent-blue"
      />
      <div className="mt-3 flex items-center justify-between gap-3">
        <button
          type="button"
          className="rounded-lg bg-accent-blue px-4 py-2 text-sm font-semibold text-white outline-none hover:bg-accent-blue-hover focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent-blue"
        >
          Send
        </button>
        <div className="flex items-center gap-2 text-ink-tertiary">
          <button
            type="button"
            aria-label="Attach file"
            className="rounded-full p-2 outline-none hover:bg-bg-hover hover:text-ink-primary focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent-blue"
          >
            <Paperclip {...iconSizeProps.md} aria-hidden="true" />
          </button>
          <button
            type="button"
            aria-label="Send later"
            className="rounded-full p-2 outline-none hover:bg-bg-hover hover:text-ink-primary focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent-blue"
          >
            <Clock {...iconSizeProps.md} aria-hidden="true" />
          </button>
        </div>
      </div>
    </section>
  );
}

function ThreadHeader({ thread }: { thread: ThreadViewResponse }) {
  const sender = primarySender(thread);
  const firstMessage = sortedMessages(thread.messages)[0];

  return (
    <header className="space-y-3">
      <h2 className="text-2xl font-bold leading-tight tracking-tight text-ink-primary sm:text-[1.75rem]">
        {thread.subject || '(no subject)'}
      </h2>
      <p className="text-sm leading-6 text-ink-secondary">
        <span className="font-semibold text-ink-primary">
          {sender ? formatParticipantName(sender) : 'Unknown'}
        </span>{' '}
        <span className="text-ink-tertiary">
          &lt;{formatParticipantEmail(sender)}&gt;
        </span>
        {firstMessage ? (
          <>
            <span className="text-ink-tertiary"> · </span>
            <time className="text-ink-tertiary">
              {formatDate(firstMessage.received_at)}
            </time>
          </>
        ) : null}
      </p>
      {thread.messages.length === 0 ? (
        <p className="sr-only">0 messages with Unknown</p>
      ) : null}
    </header>
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
  const messages = useMemo(
    () => sortedMessages(thread.messages),
    [thread.messages],
  );
  const sender = primarySender(thread);
  const [addingNoteFor, setAddingNoteFor] = useState<string | null>(null);
  const [messagePopup, setMessagePopup] = useState<{
    messageId: string;
    anchorRect: DOMRect;
  } | null>(null);
  const [notes, setNotes] = useState<LocalNote[]>([]);

  function goBack() {
    if (window.history.length > 1) {
      window.history.back();
      return;
    }

    void navigate({ to: '/imbox' });
  }

  function toggleMessagePopup(messageId: string, anchorRect: DOMRect) {
    setMessagePopup((current) =>
      current?.messageId === messageId ? null : { messageId, anchorRect },
    );
  }

  function closeMessagePopup() {
    setMessagePopup(null);
  }

  function saveNote(messageId: string, text: string) {
    setNotes((current) => [
      ...current,
      {
        id: `${messageId}-${Date.now()}`,
        messageId,
        text,
        author: 'You',
        timestamp: 'Just now',
      },
    ]);
    setAddingNoteFor(null);
  }

  return (
    <div className="mx-auto w-full max-w-[720px] space-y-8">
      <button
        type="button"
        onClick={goBack}
        className="inline-flex items-center gap-2 rounded-md text-sm font-medium text-ink-secondary outline-none hover:text-accent-blue focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent-blue"
      >
        <ArrowLeft {...iconSizeProps.sm} aria-hidden="true" />
        Back
      </button>

      <ThreadHeader thread={thread} />
      <ThreadActions thread={thread} client={client} />

      {messages.length === 0 ? (
        <StateCard
          title="No messages in this thread"
          body="The server returned the thread but did not include any messages."
        />
      ) : (
        <div className="space-y-8">
          {messages.map((message) => (
            <MessageCard
              key={message.email_id}
              message={message}
              notes={notes.filter(
                (note) => note.messageId === message.email_id,
              )}
              addingNote={addingNoteFor === message.email_id}
              popupOpen={messagePopup?.messageId === message.email_id}
              popupAnchor={
                messagePopup?.messageId === message.email_id
                  ? messagePopup.anchorRect
                  : null
              }
              onTogglePopup={toggleMessagePopup}
              onClosePopup={closeMessagePopup}
              onStartAddNote={setAddingNoteFor}
              onCancelAddNote={() => setAddingNoteFor(null)}
              onSaveNote={saveNote}
            />
          ))}
        </div>
      )}

      <MiniReplyComposer
        senderName={sender ? formatParticipantName(sender) : 'sender'}
      />
    </div>
  );
}

export function ThreadPage({ threadId, client }: ThreadPageProps) {
  const query = useThread(threadId, client);

  let reading;
  if (query.isPending) {
    reading = <LoadingState />;
  } else if (query.isError) {
    reading = (
      <ErrorState
        message={errorCopy(query.error)}
        onRetry={() => void query.refetch()}
      />
    );
  } else {
    reading = <ThreadDocument thread={query.data} client={client} />;
  }

  return <AppShell title="Thread" description={undefined} reading={reading} />;
}
