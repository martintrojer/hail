import {
  useEffect,
  useMemo,
  useRef,
  useState,
  type ChangeEvent,
  type FormEvent,
} from 'react';
import {
  type ComposeRequest,
  type ComposeResponse,
  type HailApiClient,
  type ThreadMessage,
  type ThreadParticipant,
  type ThreadViewResponse,
} from '../api/client';
import {
  defaultApiClient,
  useCreateDraftMutation,
  useDraft,
  useSendComposeMutation,
  useUpdateDraftMutation,
} from '../api/query';
import { useAuth } from '../auth/AuthProvider';
import { ArrowLeft, Paperclip, iconSizeProps } from '../components/icons';
import { useGoBack } from '../hooks/useGoBack';
import { AppShell } from '../layout/AppShell';
import { pillButtonClass } from '../lib/buttonStyles';
import { formatFullDateTime } from '../lib/dates';
import { composeErrorMessage } from '../lib/errorMessages';

interface ComposerPageProps {
  replyToThreadId?: string;
  replyAll?: boolean;
  draftId?: string;
  initialTo?: string[];
  initialSubject?: string;
  client?: HailApiClient;
}

interface ComposerForm {
  to: string;
  cc: string;
  bcc: string;
  subject: string;
  body: string;
  sendAt: string;
}

interface AttachmentDraft {
  id: string;
  name: string;
  size: number;
  type: string;
}

const autosaveIntervalMs = 5000;
const unsupportedAttachmentMessage = 'Attachments are selected, but sending and saving attachments is not supported yet. Remove them before sending, scheduling, or saving this draft.';
const lineInputClass = 'min-w-0 flex-1 border-0 bg-transparent py-3 text-base text-ink-primary outline-none placeholder:text-ink-tertiary focus:ring-0';

function splitAddresses(value: string) {
  return value.split(/[;,]/).map((address) => address.trim()).filter(Boolean);
}

function toFutureIsoDateTimeLocal(value: string, now = new Date()) {
  const date = new Date(value);
  if (!value || Number.isNaN(date.valueOf()) || date <= now) {
    return undefined;
  }
  return date.toISOString();
}

function minSendAtDateTimeLocal(now = new Date()) {
  const soon = new Date(now.getTime() + 60_000);
  const offsetMs = soon.getTimezoneOffset() * 60_000;
  return new Date(soon.getTime() - offsetMs).toISOString().slice(0, 16);
}

function fileSizeLabel(size: number) {
  if (size < 1024) return `${size} B`;
  if (size < 1024 * 1024) return `${(size / 1024).toFixed(1)} KB`;
  return `${(size / (1024 * 1024)).toFixed(1)} MB`;
}

function composeResultMessage(response: ComposeResponse) {
  return response.status === 'pending'
    ? `Scheduled for later. Draft ${response.draft_email_id} is queued.`
    : 'Sent.';
}

function uniqueEmails(emails: string[]) {
  const seen = new Set<string>();
  const unique: string[] = [];
  for (const email of emails) {
    const trimmed = email.trim();
    const normalized = trimmed.toLowerCase();
    if (!trimmed || seen.has(normalized)) continue;
    seen.add(normalized);
    unique.push(trimmed);
  }
  return unique;
}

function withoutCurrentUser(emails: string[], currentUserEmail: string | undefined) {
  const current = currentUserEmail?.trim().toLowerCase();
  return uniqueEmails(emails).filter((email) => email.toLowerCase() !== current);
}

function replySubject(subject: string) {
  return /^\s*re:/i.test(subject) ? subject : `Re: ${subject}`;
}

function formatParticipant(participant: ThreadParticipant | undefined) {
  const name = participant?.name?.trim();
  return name || participant?.email || 'Unknown sender';
}

function quotedPreview(message: ThreadMessage) {
  const preview = message.preview.trim() || 'No preview available.';
  return preview
    .split(/\r?\n/)
    .map((line) => `> ${line}`)
    .join('\n');
}

function buildReplyQuote(message: ThreadMessage) {
  return `\n\nOn ${formatFullDateTime(message.received_at, 'an earlier message')}, ${formatParticipant(message.from[0])} wrote:\n${quotedPreview(message)}`;
}

function prefillFromThread(
  thread: ThreadViewResponse,
  replyAll: boolean,
  currentUserEmail: string | undefined,
): ComposerForm | null {
  const lastMessage = thread.messages.at(-1);
  if (!lastMessage) return null;

  const senderEmail = lastMessage.from[0]?.email ?? '';
  const to = replyAll
    ? withoutCurrentUser(
        [senderEmail, ...lastMessage.to.map((participant) => participant.email)],
        currentUserEmail,
      )
    : uniqueEmails(senderEmail ? [senderEmail] : []);
  const possibleCc = (lastMessage as ThreadMessage & { cc?: ThreadParticipant[] }).cc ?? [];
  const cc = replyAll
    ? withoutCurrentUser(possibleCc.map((participant) => participant.email), currentUserEmail)
    : [];

  return {
    to: to.join(', '),
    cc: cc.join(', '),
    bcc: '',
    subject: replySubject(thread.subject),
    body: buildReplyQuote(lastMessage),
    sendAt: '',
  };
}

function placeCaretAtStart(element: HTMLTextAreaElement | null) {
  if (!element) return;
  window.requestAnimationFrame(() => {
    element.focus();
    element.setSelectionRange(0, 0);
  });
}

export function ComposerPage({ replyToThreadId, replyAll = false, draftId: initialDraftId, initialTo = [], initialSubject = '', client = defaultApiClient }: ComposerPageProps) {
  const closeComposer = useGoBack();
  const { user } = useAuth();
  const [form, setForm] = useState<ComposerForm>({
    to: initialTo.join(', '),
    cc: '',
    bcc: '',
    subject: initialSubject,
    body: '',
    sendAt: '',
  });
  const [attachments, setAttachments] = useState<AttachmentDraft[]>([]);
  const [draftId, setDraftId] = useState<string | null>(initialDraftId ?? null);
  const [dirty, setDirty] = useState(false);
  const [lastSavedAt, setLastSavedAt] = useState<Date | null>(null);
  const [successMessage, setSuccessMessage] = useState<string | null>(null);
  const [sendError, setSendError] = useState<string | null>(null);
  const [showCarbonCopyFields, setShowCarbonCopyFields] = useState(false);
  const [replyPrefillLoading, setReplyPrefillLoading] = useState(Boolean(replyToThreadId));
  const bodyRef = useRef<HTMLTextAreaElement | null>(null);
  const snapshotRef = useRef('');

  const minSendAt = useMemo(() => minSendAtDateTimeLocal(), []);
  const createDraft = useCreateDraftMutation(client);
  const draftQuery = useDraft(initialDraftId, client, { enabled: Boolean(initialDraftId) && !replyToThreadId });
  const updateDraft = useUpdateDraftMutation(client);
  const hasUnsupportedAttachments = attachments.length > 0;
  const sendCompose = useSendComposeMutation(client, {
    onSuccess: (response) => {
      setDirty(false);
      setSendError(null);
      setSuccessMessage(composeResultMessage(response));
    },
    onError: (error) => setSendError(composeErrorMessage(error, 'Message could not be sent.')),
  });

  const draftPayload = useMemo(() => ({
    to: splitAddresses(form.to),
    cc: splitAddresses(form.cc),
    bcc: splitAddresses(form.bcc),
    subject: form.subject,
    body_markdown: form.body,
    attachments: [],
  }), [form]);

  const canSaveDraft = !replyToThreadId;
  const prefillLoading = replyPrefillLoading || draftQuery.isLoading;
  const canSubmit = !prefillLoading
    && (Boolean(replyToThreadId) || draftPayload.to.length > 0)
    && (Boolean(replyToThreadId) || form.subject.trim().length > 0)
    && form.body.trim().length > 0;

  function updateField(field: keyof ComposerForm, value: string) {
    setForm((current) => ({ ...current, [field]: value }));
    setDirty(true);
    setSuccessMessage(null);
  }

  function onAttachmentsChange(event: ChangeEvent<HTMLInputElement>) {
    setAttachments(Array.from(event.target.files ?? []).map((file, index) => ({
      id: `${file.name}-${file.size}-${file.lastModified}-${index}`,
      name: file.name,
      size: file.size,
      type: file.type || 'application/octet-stream',
    })));
    setDirty(true);
    setSuccessMessage(null);
    setSendError(null);
  }

  function blockUnsupportedAttachments() {
    if (!hasUnsupportedAttachments) return false;
    setSendError(unsupportedAttachmentMessage);
    setSuccessMessage(null);
    return true;
  }

  function saveDraft() {
    if (!dirty || !canSaveDraft) return;
    if (blockUnsupportedAttachments()) return;

    const snapshot = JSON.stringify(draftPayload);
    if (snapshot === snapshotRef.current) {
      setDirty(false);
      return;
    }

    const onSuccess = (response: { draft_id: string; updated_at: string }) => {
      setDraftId(response.draft_id);
      snapshotRef.current = snapshot;
      setLastSavedAt(new Date(response.updated_at));
      setDirty(false);
    };

    if (draftId) {
      updateDraft.mutate({ draftId, request: draftPayload }, { onSuccess });
    } else {
      createDraft.mutate(draftPayload, { onSuccess });
    }
  }

  useEffect(() => {
    if (!initialDraftId || replyToThreadId || !draftQuery.data) return;

    const nextForm = {
      to: draftQuery.data.to.join(', '),
      cc: draftQuery.data.cc.join(', '),
      bcc: draftQuery.data.bcc.join(', '),
      subject: draftQuery.data.subject,
      body: draftQuery.data.body_markdown,
      sendAt: '',
    };
    setForm(nextForm);
    setDraftId(draftQuery.data.draft_id);
    setShowCarbonCopyFields(nextForm.cc.length > 0 || nextForm.bcc.length > 0);
    snapshotRef.current = JSON.stringify({
      to: draftQuery.data.to,
      cc: draftQuery.data.cc,
      bcc: draftQuery.data.bcc,
      subject: draftQuery.data.subject,
      body_markdown: draftQuery.data.body_markdown,
      attachments: [],
    });
    setDirty(false);
    setLastSavedAt(null);
    setSendError(null);
  }, [draftQuery.data, initialDraftId, replyToThreadId]);

  useEffect(() => {
    if (draftQuery.isError && initialDraftId && !replyToThreadId) {
      setSendError(composeErrorMessage(draftQuery.error, 'Draft could not be loaded.'));
    }
  }, [draftQuery.error, draftQuery.isError, initialDraftId, replyToThreadId]);

  useEffect(() => {
    if (!replyToThreadId) {
      setReplyPrefillLoading(false);
      return;
    }

    let cancelled = false;
    setReplyPrefillLoading(true);
    setSendError(null);

    client.getThread(replyToThreadId)
      .then((thread) => {
        if (cancelled) return;
        const nextForm = prefillFromThread(thread, replyAll, user?.email);
        if (nextForm) {
          setForm(nextForm);
          setShowCarbonCopyFields(nextForm.cc.length > 0);
          setDirty(false);
        }
      })
      .catch((error: unknown) => {
        if (cancelled) return;
        setSendError(composeErrorMessage(error, 'Reply details could not be loaded.'));
      })
      .finally(() => {
        if (!cancelled) setReplyPrefillLoading(false);
      });

    return () => {
      cancelled = true;
    };
  }, [client, replyAll, replyToThreadId, user?.email]);

  useEffect(() => {
    if (replyToThreadId && !replyPrefillLoading) {
      placeCaretAtStart(bodyRef.current);
    }
  }, [replyPrefillLoading, replyToThreadId]);

  useEffect(() => {
    const timer = window.setInterval(saveDraft, autosaveIntervalMs);
    return () => window.clearInterval(timer);
  });

  function buildComposeRequest(sendAt?: string): ComposeRequest {
    return {
      to: draftPayload.to,
      cc: draftPayload.cc,
      bcc: draftPayload.bcc,
      subject: draftPayload.subject,
      body_markdown: draftPayload.body_markdown,
      attachments: [],
      ...(sendAt ? { send_at: sendAt } : {}),
    };
  }

  function send(request: ComposeRequest) {
    if (blockUnsupportedAttachments()) return;
    setSendError(null);
    setSuccessMessage(null);
    sendCompose.mutate({ threadId: replyToThreadId, request });
  }

  function onSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    send(buildComposeRequest());
  }

  function sendLater() {
    const sendAt = toFutureIsoDateTimeLocal(form.sendAt);
    if (!sendAt) {
      setSendError('Choose a future send-later time.');
      return;
    }
    send(buildComposeRequest(sendAt));
  }

  const autosaveError = createDraft.error ?? updateDraft.error;
  const savingDraft = createDraft.isPending || updateDraft.isPending;

  const autosaveText = replyToThreadId
    ? 'Replies send through the thread reply API.'
    : savingDraft
      ? 'Saving draft…'
      : lastSavedAt
        ? 'Draft saved'
        : dirty
          ? 'Draft not saved yet'
          : 'Draft saved';

  return (
    <AppShell
      title={replyToThreadId ? 'Reply' : 'Compose'}
      reading={
        <section className="mx-auto flex min-h-[calc(100vh-11rem)] w-full max-w-center-column flex-col" aria-labelledby="composer-title">
          <button
            type="button"
            onClick={closeComposer}
            className="mb-8 inline-flex w-fit items-center gap-2 hail-chrome text-ink-secondary outline-none hover:text-accent-blue focus-visible:rounded-md focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent-blue"
          >
            <ArrowLeft {...iconSizeProps.sm} />
            <span>Cancel</span>
          </button>

          <div className="sr-only">
            <h2 id="composer-title">{replyToThreadId ? 'Reply to thread' : 'Compose message'}</h2>
          </div>

          {prefillLoading ? (
            <div className="flex min-h-[22rem] flex-1 items-center justify-center hail-chrome text-ink-secondary">
              {draftQuery.isLoading ? 'Loading draft…' : 'Loading reply details…'}
            </div>
          ) : (
          <form onSubmit={onSubmit} className="flex min-h-0 flex-1 flex-col">
            <div className="space-y-1">
              <div className="flex items-center gap-3 border-b border-border-hairline">
                <label htmlFor="compose-to" className="w-16 shrink-0 hail-chrome text-ink-tertiary">To</label>
                <input
                  id="compose-to"
                  type="text"
                  value={form.to}
                  onChange={(event) => updateField('to', event.target.value)}
                  placeholder="alice@example.com, bob@example.com"
                  className={lineInputClass}
                  autoComplete="email"
                  autoFocus={!replyToThreadId && !initialDraftId}
                />
                <button
                  type="button"
                  onClick={() => setShowCarbonCopyFields((shown) => !shown)}
                  className="shrink-0 hail-chrome text-ink-tertiary outline-none hover:text-accent-blue focus-visible:rounded-md focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent-blue"
                  aria-expanded={showCarbonCopyFields}
                  aria-controls="compose-carbon-copy-fields"
                >
                  Cc / Bcc
                </button>
              </div>

              <div id="compose-carbon-copy-fields" className={showCarbonCopyFields ? 'grid gap-1' : 'hidden'}>
                <div className="flex items-center gap-3 border-b border-border-hairline">
                  <label htmlFor="compose-cc" className="w-16 shrink-0 hail-chrome text-ink-tertiary">Cc</label>
                  <input
                    id="compose-cc"
                    type="text"
                    value={form.cc}
                    onChange={(event) => updateField('cc', event.target.value)}
                    className={lineInputClass}
                    autoComplete="email"
                  />
                </div>
                <div className="flex items-center gap-3 border-b border-border-hairline">
                  <label htmlFor="compose-bcc" className="w-16 shrink-0 hail-chrome text-ink-tertiary">Bcc</label>
                  <input
                    id="compose-bcc"
                    type="text"
                    value={form.bcc}
                    onChange={(event) => updateField('bcc', event.target.value)}
                    className={lineInputClass}
                    autoComplete="email"
                  />
                </div>
              </div>

              <div className="flex items-center gap-3 border-b border-border-hairline">
                <label htmlFor="compose-subject" className="w-16 shrink-0 hail-chrome text-ink-tertiary">Subject</label>
                <input
                  id="compose-subject"
                  type="text"
                  value={form.subject}
                  onChange={(event) => updateField('subject', event.target.value)}
                  className={lineInputClass}
                  placeholder="Subject"
                />
              </div>
            </div>

            <div className="mt-8 flex min-h-[22rem] flex-1 flex-col">
              <label htmlFor="compose-body" className="sr-only">Body</label>
              <textarea
                ref={bodyRef}
                id="compose-body"
                value={form.body}
                onChange={(event) => updateField('body', event.target.value)}
                className="min-h-[22rem] flex-1 resize-none border-0 bg-transparent text-base leading-relaxed text-ink-primary outline-none placeholder:text-ink-tertiary focus:ring-0"
                placeholder="Write your email…"
                autoFocus={Boolean(replyToThreadId)}
              />
            </div>

            <div className="mt-3 flex min-h-6 items-center justify-between gap-4">
              <div className="flex items-center gap-3 text-ink-tertiary">
                <label htmlFor="compose-attachments" className="inline-flex cursor-pointer items-center gap-2 hail-chrome outline-none hover:text-accent-blue">
                  <Paperclip {...iconSizeProps.sm} />
                  <span>Attachments</span>
                </label>
                <input id="compose-attachments" type="file" multiple onChange={onAttachmentsChange} className="sr-only" />
                <label htmlFor="compose-send-at" className="sr-only">Send later</label>
                <input
                  id="compose-send-at"
                  type="datetime-local"
                  value={form.sendAt}
                  min={minSendAt}
                  onChange={(event) => updateField('sendAt', event.target.value)}
                  className="w-40 border-0 bg-transparent hail-chrome text-ink-tertiary outline-none focus:ring-0"
                />
              </div>
              <p className={`hail-badge text-ink-tertiary transition-opacity duration-500 ${savingDraft || lastSavedAt || dirty || replyToThreadId ? 'opacity-100' : 'opacity-0'}`}>
                {autosaveText}
              </p>
            </div>

            {attachments.length > 0 ? <AttachmentNotice attachments={attachments} /> : null}

            <div className="mt-8 flex items-center gap-4 border-t border-border-hairline pt-5">
              <button type="submit" disabled={!canSubmit || hasUnsupportedAttachments || sendCompose.isPending} className={pillButtonClass('primary', 'md')}>{sendCompose.isPending && !form.sendAt ? 'Sending…' : 'Send now'}</button>
              <button type="button" onClick={sendLater} disabled={!canSubmit || !form.sendAt || hasUnsupportedAttachments || sendCompose.isPending} className="hail-chrome text-ink-secondary outline-none hover:text-accent-blue focus-visible:rounded-md focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent-blue disabled:cursor-not-allowed disabled:opacity-50">{sendCompose.isPending && form.sendAt ? 'Scheduling…' : 'Send later'}</button>
              {!replyToThreadId ? <button type="button" onClick={saveDraft} disabled={!dirty || savingDraft || !canSaveDraft || hasUnsupportedAttachments} className="hail-chrome text-ink-tertiary outline-none hover:text-accent-blue focus-visible:rounded-md focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent-blue disabled:cursor-not-allowed disabled:opacity-50">{savingDraft ? 'Saving…' : 'Save draft'}</button> : null}
              <button type="button" onClick={closeComposer} className="ml-auto hail-chrome text-ink-tertiary outline-none hover:text-accent-red focus-visible:rounded-md focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent-blue">Discard</button>
            </div>

            <div className="mt-4 space-y-2">
              {autosaveError ? <Status kind="warn" message={composeErrorMessage(autosaveError, 'Draft autosave failed.')} /> : null}
              {sendError ? <Status kind="error" message={sendError} /> : null}
              {successMessage ? <Status kind="success" message={successMessage} /> : null}
            </div>
          </form>
          )}
        </section>
      }
    />
  );
}

function AttachmentNotice({ attachments }: { attachments: AttachmentDraft[] }) {
  return (
    <div className="mt-4 rounded-lg border border-border-menu bg-bg-banner p-3 hail-chrome text-ink-secondary">
      <p className="font-semibold text-ink-primary">Attachments are not supported for sending or saving yet.</p>
      <ul className="mt-2 space-y-1 text-ink-secondary">
        {attachments.map((attachment) => <li key={attachment.id}>{attachment.name} · {fileSizeLabel(attachment.size)} · {attachment.type}</li>)}
      </ul>
    </div>
  );
}

function Status({ kind, message }: { kind: 'error' | 'success' | 'warn'; message: string }) {
  const className = kind === 'success'
    ? 'border-border-menu bg-bg-surface text-ink-primary'
    : kind === 'warn'
      ? 'border-border-menu bg-bg-banner text-ink-primary'
      : 'border-accent-red/40 bg-bg-surface text-accent-red';
  return <p className={`rounded-lg border px-3 py-2 hail-chrome ${className}`}>{message}</p>;
}
