import { useNavigate } from '@tanstack/react-router';
import { useEffect, useMemo, useRef, useState } from 'react';
import {
  HailApiClient,
  type ThreadMessage,
  type ThreadNote,
  type ThreadVerbResponse,
  type ThreadViewResponse,
} from '../api/client';
import { useApiClient } from '../api/ApiClientProvider';
import {
  useArchiveThreadMutation,
  useBubbleUpMutation,
  useClassifyThreadMutation,
  useReplyLaterThreadMutation,
  useSetAsideThreadMutation,
  useSpamThreadMutation,
  useThread,
  useTrashThreadMutation,
} from '../api/query';
import { AddNoteForm } from '../components/AddNoteForm';
import { AttachmentList } from '../components/AttachmentList';
import { BubbleUpSubmenu } from '../components/BubbleUpSubmenu';
import { EmailFrame } from '../components/EmailFrame';
import { ErrorState } from '../components/ErrorState';
import { InlineNote, type InlineNoteProps } from '../components/InlineNote';
import {
  ArrowLeft,
  MoreHorizontal,
  ShieldOff,
  StickyNote,
} from '../components/icons';
import { LabelChips } from '../components/LabelChips';
import { ThreadLabelPicker } from '../components/ThreadLabelPicker';
import { LoadingState } from '../components/LoadingState';
import { StateCard } from '../components/StateCard';
import { Alert, AlertAction, AlertDescription } from '../components/ui/alert';
import { Badge } from '../components/ui/badge';
import { Button } from '../components/ui/button';
import {
  Card,
  CardContent,
  CardHeader,
  CardTitle,
} from '../components/ui/card';
import { Separator } from '../components/ui/separator';
import { MessageActionPopup } from '../components/MessageActionPopup';
import { useUndoToast } from '../components/UndoToastProvider';
import { useGoBack } from '../hooks/useGoBack';
import { useKeyboardShortcuts } from '../hooks/useKeyboardShortcuts';
import { AppShell } from '../layout/AppShell';
import { formatFullDateTime } from '../lib/dates';
import { actionErrorMessage, threadErrorMessage } from '../lib/errorMessages';
import {
  formatParticipantEmail,
  formatParticipantList,
  formatParticipantName,
} from '../lib/participants';

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

const remoteImagesStorageKeyPrefix = 'hail.thread.remote-images.';

function remoteImagesStorageKey(threadId: string, messageId: string) {
  return `${remoteImagesStorageKeyPrefix}${threadId}.${messageId}`;
}

function storedRemoteImagesPreference(threadId: string, messageId: string) {
  if (typeof window === 'undefined') return false;
  try {
    return (
      window.localStorage.getItem(
        remoteImagesStorageKey(threadId, messageId),
      ) === '1'
    );
  } catch {
    return false;
  }
}

function storeRemoteImagesPreference(
  threadId: string,
  messageId: string,
  enabled: boolean,
) {
  if (typeof window === 'undefined') return;
  try {
    const key = remoteImagesStorageKey(threadId, messageId);
    if (enabled) {
      window.localStorage.setItem(key, '1');
    } else {
      window.localStorage.removeItem(key);
    }
  } catch {
    // localStorage may be unavailable in hardened/private browser contexts.
  }
}

function TrackerBadge({ message }: { message: ThreadMessage }) {
  const count = message.blocked_trackers.length;
  if (count === 0) {
    return null;
  }

  return (
    <Badge
      variant="secondary"
      className="max-w-40 truncate"
      title={message.blocked_trackers
        .map((tracker) => tracker.reason)
        .join('\n')}
    >
      {count} tracker{count === 1 ? '' : 's'} blocked
    </Badge>
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

function isUnreadMessage(message: ThreadMessage) {
  return Boolean((message as ThreadMessage & { unread?: boolean }).unread);
}

function initialActiveMessageId(messages: ThreadMessage[]) {
  for (let index = messages.length - 1; index >= 0; index -= 1) {
    if (isUnreadMessage(messages[index])) {
      return messages[index].email_id;
    }
  }

  return messages.at(-1)?.email_id ?? null;
}

function MessageCard({
  threadId,
  message,
  notes,
  addingNote,
  popupOpen,
  onTogglePopup,
  onClosePopup,
  onPopupAction,
  onCancelAddNote,
  onSaveNote,
  hiddenActions,
  actionBusy,
  active,
  messageRef,
}: {
  threadId: string;
  message: ThreadMessage;
  notes: LocalNote[];
  addingNote: boolean;
  popupOpen: boolean;
  actionBusy: boolean;
  active: boolean;
  messageRef: (node: HTMLElement | null) => void;
  onTogglePopup: (messageId: string) => void;
  onClosePopup: () => void;
  onPopupAction: (
    message: ThreadMessage,
    action: string,
    payload?: unknown,
  ) => void;
  onCancelAddNote: () => void;
  onSaveNote: (messageId: string, text: string) => void;
  hiddenActions?: string[];
}) {
  const sender = firstSender(message);
  const remoteImagesAvailable =
    message.html_with_remote_images !== message.html;
  const [showRemoteImages, setShowRemoteImages] = useState(() =>
    storedRemoteImagesPreference(threadId, message.email_id),
  );
  const renderedHtml = showRemoteImages
    ? message.html_with_remote_images
    : message.html;

  useEffect(() => {
    if (!remoteImagesAvailable && showRemoteImages) {
      setShowRemoteImages(false);
      storeRemoteImagesPreference(threadId, message.email_id, false);
    }
  }, [message.email_id, remoteImagesAvailable, showRemoteImages, threadId]);

  function toggleRemoteImages() {
    setShowRemoteImages((current) => {
      const next = !current;
      storeRemoteImagesPreference(threadId, message.email_id, next);
      return next;
    });
  }

  function togglePopup() {
    onTogglePopup(message.email_id);
  }

  function handlePopupAction(action: string, payload?: unknown) {
    onPopupAction(message, action, payload);
  }

  return (
    <article
      ref={messageRef}
      data-email-id={message.email_id}
      aria-current={active ? 'true' : undefined}
      className={`rounded-lg border-l-2 py-5 pl-3 pr-2 transition-colors ${active ? 'border-l-primary bg-primary/5' : 'border-l-transparent'}`}
    >
      <header className="flex items-start gap-3">
        <div className="grid size-8 shrink-0 place-items-center rounded-full bg-muted text-xs font-semibold text-muted-foreground">
          <span aria-hidden="true">{participantInitial(sender)}</span>
        </div>
        <div className="min-w-0 flex-1">
          <div className="flex items-start justify-between gap-3">
            <div className="min-w-0">
              <p className="truncate font-semibold text-foreground">
                {sender ? formatParticipantName(sender) : 'Unknown sender'}
              </p>
              <p className="mt-0.5 truncate text-sm text-muted-foreground">
                To {formatParticipantList(message.to)} ·{' '}
                <time>{formatFullDateTime(message.received_at)}</time>
              </p>
            </div>
            <div className="flex shrink-0 items-center gap-2">
              <TrackerBadge message={message} />
              <MessageActionPopup
                open={popupOpen}
                onClose={onClosePopup}
                onAction={handlePopupAction}
                hiddenActions={hiddenActions}
                trigger={
                  <Button
                    type="button"
                    variant="ghost"
                    size="icon-xs"
                    aria-label="Message actions"
                    aria-haspopup="menu"
                    aria-expanded={popupOpen}
                    disabled={actionBusy}
                    onMouseDown={(event) => event.stopPropagation()}
                    onClick={togglePopup}
                  >
                    <MoreHorizontal aria-hidden="true" />
                  </Button>
                }
              />
            </div>
          </div>

          {remoteImagesAvailable ? (
            <Alert className="mt-4 pr-40">
              <ShieldOff aria-hidden="true" />
              <AlertDescription>
                Remote images are hidden by default. Tracking pixels stay
                blocked.
              </AlertDescription>
              <AlertAction>
                <Button
                  type="button"
                  variant="outline"
                  size="xs"
                  onClick={toggleRemoteImages}
                >
                  {showRemoteImages
                    ? 'Hide remote images'
                    : 'Show remote images'}
                </Button>
              </AlertAction>
            </Alert>
          ) : null}

          {notes.length > 0 ? (
            <div className="mt-5 flex flex-col gap-3">
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

          {renderedHtml.trim().length > 0 ? (
            <EmailFrame
              html={renderedHtml}
              title={`Email body from ${sender ? formatParticipantName(sender) : 'Unknown sender'}`}
              className="mt-5"
            />
          ) : (
            <p className="mt-5 whitespace-pre-wrap text-base leading-relaxed text-foreground">
              {message.preview || 'This message has no renderable body.'}
            </p>
          )}

          <AttachmentList items={message.attachments} />

          {addingNote ? (
            <Card
              size="sm"
              className="mt-5 rounded-r-lg border-l-4 border-l-primary"
            >
              <CardHeader>
                <CardTitle className="flex items-center gap-2 text-sm">
                  <StickyNote aria-hidden="true" />
                  Add note
                </CardTitle>
              </CardHeader>
              <CardContent>
                <AddNoteForm
                  onSave={(text) => onSaveNote(message.email_id, text)}
                  onCancel={onCancelAddNote}
                />
              </CardContent>
            </Card>
          ) : null}
        </div>
      </header>
    </article>
  );
}

function ThreadHeader({
  thread,
  client,
}: {
  thread: ThreadViewResponse;
  client: HailApiClient;
}) {
  const sender = primarySender(thread);
  const firstMessage = sortedMessages(thread.messages)[0];

  return (
    <header className="flex flex-col gap-3">
      <h2 className="text-2xl font-bold leading-tight tracking-tight text-foreground sm:text-[1.75rem]">
        {thread.subject || '(no subject)'}
      </h2>
      <p className="text-sm leading-6 text-muted-foreground">
        <span className="font-semibold text-foreground">
          {sender ? formatParticipantName(sender) : 'Unknown'}
        </span>{' '}
        <span className="text-muted-foreground">
          &lt;{formatParticipantEmail(sender)}&gt;
        </span>
        {firstMessage ? (
          <>
            <span className="text-muted-foreground"> · </span>
            <time className="text-muted-foreground">
              {formatFullDateTime(firstMessage.received_at)}
            </time>
          </>
        ) : null}
      </p>
      <div
        className="flex flex-wrap items-center gap-2"
        aria-label="Thread labels"
      >
        {thread.labels.length > 0 ? (
          <LabelChips
            labels={thread.labels}
            className="flex min-w-0 flex-wrap items-center gap-1.5"
          />
        ) : (
          <span className="text-xs text-muted-foreground">No labels</span>
        )}
        <ThreadLabelPicker
          threadId={thread.thread_id}
          assignedLabels={thread.labels}
          client={client}
        />
      </div>
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
  const goBack = useGoBack();
  const { showToast } = useUndoToast();
  const messages = useMemo(
    () => sortedMessages(thread.messages),
    [thread.messages],
  );
  const hiddenPopupActions =
    sourceView === 'set-aside' || sourceView === 'reply-later'
      ? ['bubble-up', 'set-aside', 'reply-later']
      : [];
  const [addingNoteFor, setAddingNoteFor] = useState<string | null>(null);
  const [messagePopup, setMessagePopup] = useState<string | null>(null);
  const [bubbleUpOpen, setBubbleUpOpen] = useState(false);
  const [notes, setNotes] = useState<LocalNote[]>(() =>
    thread.notes.map(toLocalNote),
  );
  const [actionError, setActionError] = useState<string | null>(null);
  const messageRefs = useRef(new Map<string, HTMLElement>());
  const [activeEmailId, setActiveEmailId] = useState<string | null>(() =>
    initialActiveMessageId(messages),
  );

  const setAside = useSetAsideThreadMutation(client);
  const replyLater = useReplyLaterThreadMutation(client);
  const trash = useTrashThreadMutation(client);
  const spam = useSpamThreadMutation(client);
  const archive = useArchiveThreadMutation(client);
  const classify = useClassifyThreadMutation(client);
  const bubbleUp = useBubbleUpMutation(client);
  const actionBusy =
    setAside.isPending ||
    replyLater.isPending ||
    trash.isPending ||
    spam.isPending ||
    archive.isPending ||
    classify.isPending ||
    bubbleUp.isPending;

  useEffect(() => {
    setNotes(thread.notes.map(toLocalNote));
  }, [thread.notes]);

  useEffect(() => {
    const messageIds = new Set(messages.map((message) => message.email_id));
    if (!activeEmailId || !messageIds.has(activeEmailId)) {
      setActiveEmailId(initialActiveMessageId(messages));
    }
  }, [activeEmailId, messages]);

  useEffect(() => {
    if (!activeEmailId) {
      return;
    }

    messageRefs.current
      .get(activeEmailId)
      ?.scrollIntoView({ block: 'nearest' });
  }, [activeEmailId]);

  function setMessageRef(messageId: string) {
    return (node: HTMLElement | null) => {
      if (node) {
        messageRefs.current.set(messageId, node);
      } else {
        messageRefs.current.delete(messageId);
      }
    };
  }

  function toggleMessagePopup(messageId: string) {
    setMessagePopup((current) => (current === messageId ? null : messageId));
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

  async function runThreadAction<T>(operation: () => Promise<T>) {
    setActionError(null);
    try {
      return await operation();
    } catch (error) {
      const normalizedError =
        error instanceof Error ? error : new Error('Thread action failed');
      setActionError(actionErrorMessage(normalizedError, 'Thread action'));
      return null;
    }
  }

  async function handleBubbleUpSelect(option: string) {
    const isoDate = bubbleUpOptionToIso(option);
    const response = await runThreadAction(() =>
      bubbleUp.mutateAsync({
        threadId: thread.thread_id,
        request: { at: isoDate },
      }),
    );
    if (!response) {
      return;
    }
    showToast({
      message: `Thread will bubble up at ${formatFullDateTime(isoDate)}`,
    });
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
        setActionError(null);
        setAddingNoteFor(message.email_id);
        return;
      case 'set-aside': {
        const response = await runThreadAction(() =>
          setAside.mutateAsync({ threadId: thread.thread_id }),
        );
        if (!response) {
          return;
        }
        showUndoToast(
          'Thread added to Set Aside.',
          response,
          'Set Aside undone.',
        );
        goBack();
        return;
      }
      case 'reply-later': {
        const response = await runThreadAction(() =>
          replyLater.mutateAsync({ threadId: thread.thread_id }),
        );
        if (!response) {
          return;
        }
        showUndoToast(
          'Thread added to Reply Later.',
          response,
          'Reply Later undone.',
        );
        goBack();
        return;
      }
      case 'trash': {
        const response = await runThreadAction(() =>
          trash.mutateAsync({ threadId: thread.thread_id }),
        );
        if (!response) {
          return;
        }
        showUndoToast('Thread moved to trash.', response, 'Trash undone.');
        goBack();
        return;
      }
      case 'archive': {
        const response = await runThreadAction(() =>
          archive.mutateAsync({ threadId: thread.thread_id }),
        );
        if (!response) {
          return;
        }
        goBack();
        return;
      }
      case 'reply':
        setActionError(null);
        void navigate({
          to: '/compose',
          search: {
            replyTo: thread.thread_id,
            replyAll: false,
            in_reply_to: message.email_id,
          },
        });
        return;
      case 'reply-all':
        setActionError(null);
        void navigate({
          to: '/compose',
          search: {
            replyTo: thread.thread_id,
            replyAll: true,
            in_reply_to: message.email_id,
          },
        });
        return;
      case 'forward':
        setActionError(null);
        void navigate({
          to: '/compose',
          search: { forward: thread.thread_id, in_reply_to: message.email_id },
        });
        return;
      case 'move-to': {
        if (
          payload !== 'imbox' &&
          payload !== 'feed' &&
          payload !== 'papertrail'
        ) {
          showToast({ message: 'Move target not supported.' });
          return;
        }
        const response = await runThreadAction(() =>
          classify.mutateAsync({ threadId: thread.thread_id, to: payload }),
        );
        if (!response) {
          return;
        }
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
        setActionError(null);
        setBubbleUpOpen(true);
        return;
      case 'mark-spam': {
        const response = await runThreadAction(() =>
          spam.mutateAsync({ threadId: thread.thread_id }),
        );
        if (!response) {
          return;
        }
        showUndoToast(
          'Thread marked as spam.',
          response,
          'Spam action undone.',
        );
        goBack();
        return;
      }
      default:
        return;
    }
  }

  function activeMessageForShortcut() {
    return (
      messages.find((message) => message.email_id === activeEmailId) ??
      messages.at(-1) ??
      null
    );
  }

  function moveActiveMessage(delta: number) {
    if (messages.length === 0) {
      return;
    }

    const currentIndex = messages.findIndex(
      (message) => message.email_id === activeEmailId,
    );
    const fallbackIndex = messages.length - 1;
    const nextIndex = Math.min(
      messages.length - 1,
      Math.max(0, (currentIndex === -1 ? fallbackIndex : currentIndex) + delta),
    );
    setActiveEmailId(messages[nextIndex].email_id);
  }

  function handleReplyShortcut() {
    const message = activeMessageForShortcut();
    if (!message) {
      return;
    }

    setActionError(null);
    void navigate({
      to: '/compose',
      search: {
        replyTo: thread.thread_id,
        replyAll: false,
        in_reply_to: message.email_id,
      },
    });
  }

  function handleReplyAllShortcut() {
    const message = activeMessageForShortcut();
    if (!message) {
      return;
    }

    setActionError(null);
    void navigate({
      to: '/compose',
      search: {
        replyTo: thread.thread_id,
        replyAll: true,
        in_reply_to: message.email_id,
      },
    });
  }

  function handleForwardShortcut() {
    const message = activeMessageForShortcut();
    if (!message) {
      return;
    }

    setActionError(null);
    void navigate({
      to: '/compose',
      search: { forward: thread.thread_id, in_reply_to: message.email_id },
    });
  }

  function handleAddNoteShortcut() {
    const message = activeMessageForShortcut();
    if (!message) {
      return;
    }

    setActionError(null);
    setAddingNoteFor(message.email_id);
  }

  function handleThreadShortcut(
    action: 'archive' | 'trash' | 'set-aside' | 'reply-later',
  ) {
    const message = activeMessageForShortcut();
    if (!message || actionBusy) {
      return;
    }

    void handlePopupAction(message, action);
  }

  function openActionMenuShortcut() {
    const activeMessage = activeMessageForShortcut();
    if (!activeMessage) {
      return;
    }

    const activeElement = messageRefs.current.get(activeMessage.email_id);
    const button = activeElement?.querySelector<HTMLButtonElement>(
      '[aria-label="Message actions"]',
    );
    button?.click();
  }

  useKeyboardShortcuts({
    onNextThread: () => moveActiveMessage(1),
    onPreviousThread: () => moveActiveMessage(-1),
    onReply: handleReplyShortcut,
    onReplyAll: handleReplyAllShortcut,
    onForward: handleForwardShortcut,
    onAddNote: handleAddNoteShortcut,
    onOpenActionMenu: openActionMenuShortcut,
    onArchive: () => handleThreadShortcut('archive'),
    onTrash: () => handleThreadShortcut('trash'),
    onSetAside: () => handleThreadShortcut('set-aside'),
    onReplyLater: () => handleThreadShortcut('reply-later'),
    onGoBack: goBack,
    onEscape: goBack,
  });

  async function saveNote(messageId: string, text: string) {
    const note = await client.createThreadNote(thread.thread_id, {
      email_id: messageId,
      body: text,
    });
    setNotes((current) => [...current, toLocalNote(note)]);
    setAddingNoteFor(null);
  }

  return (
    <div className="flex flex-col gap-6">
      <Button
        type="button"
        variant="ghost"
        size="sm"
        onClick={goBack}
        className="w-fit"
      >
        <ArrowLeft data-icon="inline-start" aria-hidden="true" />
        Back
      </Button>

      <ThreadHeader thread={thread} client={client} />

      {actionError ? (
        <Alert variant="destructive">
          <AlertDescription>{actionError}</AlertDescription>
        </Alert>
      ) : null}

      {messages.length === 0 ? (
        <StateCard
          title="No messages in this thread"
          body="The server returned the thread but did not include any messages."
        />
      ) : (
        <div className="flex flex-col">
          {messages.map((message, index) => (
            <div key={message.email_id}>
              {index > 0 ? <Separator /> : null}
              <MessageCard
                threadId={thread.thread_id}
                message={message}
                notes={notes.filter(
                  (note) => note.messageId === message.email_id,
                )}
                addingNote={addingNoteFor === message.email_id}
                popupOpen={messagePopup === message.email_id}
                actionBusy={actionBusy}
                active={activeEmailId === message.email_id}
                messageRef={setMessageRef(message.email_id)}
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
            </div>
          ))}
        </div>
      )}

      <BubbleUpSubmenu
        open={bubbleUpOpen}
        onClose={() => setBubbleUpOpen(false)}
        onSelect={(option) => {
          void handleBubbleUpSelect(option);
        }}
      />
    </div>
  );
}

export function ThreadPage({ threadId, client, sourceView }: ThreadPageProps) {
  const contextClient = useApiClient();
  const apiClient = client ?? contextClient;
  const query = useThread(threadId, apiClient);

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
        message={threadErrorMessage(query.error)}
        onRetry={() => void query.refetch()}
      />
    );
  } else {
    reading = (
      <ThreadDocument
        thread={query.data}
        client={apiClient}
        sourceView={sourceView}
      />
    );
  }

  return (
    <AppShell
      title="Thread"
      description={undefined}
      reading={reading}
      contentLayout="reading"
    />
  );
}
