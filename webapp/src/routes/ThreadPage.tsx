import { useNavigate } from '@tanstack/react-router';
import { useEffect, useMemo, useRef, useState, type MouseEvent } from 'react';
import {
  HailApiClient,
  HailApiError,
  type ThreadMessage,
  type ThreadNote,
  type ThreadVerbResponse,
  type ThreadViewResponse,
} from '../api/client';
import {
  useThread,
  defaultApiClient,
} from '../api/query';
import { AddNoteForm } from '../components/AddNoteForm';
import { BubbleUpSubmenu } from '../components/BubbleUpSubmenu';
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

interface ThreadPageProps {
  threadId: string;
  client?: Parameters<typeof useThread>[1];
  /** Source view the thread was opened from (e.g. 'set-aside', 'reply-later'). */
  sourceView?: string;
}

interface LocalNote extends InlineNoteProps {
  id: number;
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

function noteTimestamp(value: string) {
  const date = new Date(value);
  if (Number.isNaN(date.valueOf())) {
    return value;
  }

  return new Intl.DateTimeFormat(undefined, {
    dateStyle: 'medium',
    timeStyle: 'short',
  }).format(date);
}

function bubbleUpOptionToIso(option: string) {
  const now = new Date();
  const target = new Date(now);

  switch (option) {
    case 'Later today':
      if (now.getHours() >= 14) {
        target.setTime(now.getTime() + 3 * 60 * 60 * 1000);
      } else {
        target.setHours(17, 0, 0, 0);
      }
      break;
    case 'Tomorrow morning':
      target.setDate(now.getDate() + 1);
      target.setHours(9, 0, 0, 0);
      break;
    case 'This weekend': {
      const daysUntilSaturday = (6 - now.getDay() + 7) % 7 || 7;
      target.setDate(now.getDate() + daysUntilSaturday);
      target.setHours(10, 0, 0, 0);
      break;
    }
    case 'Next week': {
      const daysUntilMonday = (1 - now.getDay() + 7) % 7 || 7;
      target.setDate(now.getDate() + daysUntilMonday);
      target.setHours(9, 0, 0, 0);
      break;
    }
    case 'Pick a date…':
    default:
      target.setTime(now.getTime() + 24 * 60 * 60 * 1000);
      break;
  }

  return target.toISOString();
}

function toLocalNote(note: ThreadNote): LocalNote {
  return {
    id: note.id,
    messageId: note.email_id,
    text: note.body,
    author: 'You',
    timestamp: noteTimestamp(note.created_at),
  };
}


function MessageCard({
  message,
  notes,
  addingNote,
  popupOpen,
  popupAnchor,
  onTogglePopup,
  onClosePopup,
  onPopupAction,
  onCancelAddNote,
  onSaveNote,
  hiddenActions,
}: {
  message: ThreadMessage;
  notes: LocalNote[];
  addingNote: boolean;
  popupOpen: boolean;
  popupAnchor: DOMRect | null;
  onTogglePopup: (messageId: string, anchorRect: DOMRect) => void;
  onClosePopup: () => void;
  onPopupAction: (message: ThreadMessage, action: string, payload?: unknown) => void;
  onCancelAddNote: () => void;
  onSaveNote: (messageId: string, text: string) => void;
  hiddenActions?: string[];
}) {
  const sender = firstSender(message);

  function togglePopup(event: MouseEvent<HTMLButtonElement>) {
    onTogglePopup(message.email_id, event.currentTarget.getBoundingClientRect());
  }

  function handlePopupAction(action: string, payload?: unknown) {
    onPopupAction(message, action, payload);
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
                hiddenActions={hiddenActions}
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
          className="rounded-full bg-accent-blue px-4 py-1.5 text-xs font-semibold text-white outline-none hover:bg-accent-blue-hover focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent-blue"
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
  sourceView,
}: {
  thread: ThreadViewResponse;
  client: HailApiClient;
  sourceView?: string;
}) {
  const navigate = useNavigate();
  const { showToast } = useUndoToast();
  const messages = useMemo(
    () => sortedMessages(thread.messages),
    [thread.messages],
  );
  const sender = primarySender(thread);
  const hiddenPopupActions = sourceView === 'set-aside' || sourceView === 'reply-later'
    ? ['bubble-up', 'set-aside', 'reply-later']
    : [];
  const [addingNoteFor, setAddingNoteFor] = useState<string | null>(null);
  const [messagePopup, setMessagePopup] = useState<{
    messageId: string;
    anchorRect: DOMRect;
  } | null>(null);
  const [bubbleUpOpen, setBubbleUpOpen] = useState(false);
  const [bubbleUpAnchor, setBubbleUpAnchor] = useState<DOMRect | null>(null);
  const [notes, setNotes] = useState<LocalNote[]>(() => thread.notes.map(toLocalNote));

  useEffect(() => {
    setNotes(thread.notes.map(toLocalNote));
  }, [thread.notes]);

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

  function showUndoToast(
    message: string,
    response: ThreadVerbResponse,
    undoSuccessMessage: string,
  ) {
    showToast({
      message,
      undo: response.undo ? { id: response.undo.id } : null,
      undoSuccessMessage,
    });
  }

  async function handleBubbleUpSelect(option: string) {
    const isoDate = bubbleUpOptionToIso(option);
    await client.bubbleUpThread(thread.thread_id, { at: isoDate });
    showToast({ message: `Thread will bubble up at ${formatDate(isoDate)}` });
    goBack();
  }

  async function handlePopupAction(
    message: ThreadMessage,
    action: string,
    payload?: unknown,
  ) {
    closeMessagePopup();

    switch (action) {
      case 'add-note':
        setAddingNoteFor(message.email_id);
        return;
      case 'set-aside': {
        const response = await client.setAsideThread(thread.thread_id);
        showUndoToast('Thread added to Set Aside.', response, 'Set Aside undone.');
        goBack();
        return;
      }
      case 'reply-later': {
        const response = await client.replyLaterThread(thread.thread_id);
        showUndoToast('Thread added to Reply Later.', response, 'Reply Later undone.');
        goBack();
        return;
      }
      case 'trash': {
        const response = await client.trashThread(thread.thread_id);
        showUndoToast('Thread moved to trash.', response, 'Trash undone.');
        goBack();
        return;
      }
      case 'archive':
        await client.archiveThread(thread.thread_id);
        goBack();
        return;
      case 'reply':
        void navigate({ to: '/compose', search: { replyTo: thread.thread_id, replyAll: false } });
        return;
      case 'reply-all':
        void navigate({ to: '/compose', search: { replyTo: thread.thread_id, replyAll: true } });
        return;
      case 'forward':
        void navigate({ to: '/compose', search: { forward: message.email_id } });
        return;
      case 'move-to': {
        if (payload !== 'imbox' && payload !== 'feed' && payload !== 'papertrail') {
          showToast({ message: 'Move target not supported.' });
          return;
        }
        const response = await client.moveThread(thread.thread_id, payload);
        const labels = {
          imbox: 'Imbox',
          feed: 'Feed',
          papertrail: 'Paper Trail',
        };
        showUndoToast(
          `Moved thread to ${labels[payload]}.`,
          response,
          'Thread move undone.',
        );
        goBack();
        return;
      }
      case 'bubble-up':
        setBubbleUpAnchor(messagePopup?.anchorRect ?? null);
        setBubbleUpOpen(true);
        return;
      case 'mark-spam':
        showToast({ message: 'Spam reporting coming soon.' });
        return;
      default:
        return;
    }
  }

  async function saveNote(messageId: string, text: string) {
    const note = await client.createThreadNote(thread.thread_id, {
      email_id: messageId,
      body: text,
    });
    setNotes((current) => [...current, toLocalNote(note)]);
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
              onPopupAction={(message, action, payload) => {
                void handlePopupAction(message, action, payload);
              }}
              onCancelAddNote={() => setAddingNoteFor(null)}
              onSaveNote={(messageId, text) => {
                void saveNote(messageId, text);
              }}
              hiddenActions={hiddenPopupActions}
            />
          ))}
        </div>
      )}

      <MiniReplyComposer
        senderName={sender ? formatParticipantName(sender) : 'sender'}
      />
      <BubbleUpSubmenu
        open={bubbleUpOpen}
        anchorRect={bubbleUpAnchor}
        onClose={() => setBubbleUpOpen(false)}
        onSelect={(option) => {
          void handleBubbleUpSelect(option);
        }}
      />
    </div>
  );
}

export function ThreadPage({ threadId, client, sourceView }: ThreadPageProps) {
  const query = useThread(threadId, client);
  const apiClient = client ?? defaultApiClient;

  // Mark thread as read when it loads successfully
  useEffect(() => {
    if (query.isSuccess) {
      apiClient.markThread(threadId, true).catch(() => {
        // Silently ignore mark-as-read failures
      });
    }
  }, [query.isSuccess, threadId, apiClient]);

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
    reading = <ThreadDocument thread={query.data} client={apiClient} sourceView={sourceView} />;
  }

  return <AppShell title="Thread" description={undefined} reading={reading} />;
}
