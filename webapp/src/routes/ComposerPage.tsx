import {
  useEffect,
  useMemo,
  useRef,
  useState,
  type ChangeEvent,
  type FormEvent,
} from 'react';
import { useNavigate } from '@tanstack/react-router';
import { HailApiError, type HailApiClient, type ComposeRequest, type ComposeResponse } from '../api/client';
import {
  defaultApiClient,
  useCreateDraftMutation,
  useSendComposeMutation,
  useUpdateDraftMutation,
} from '../api/query';
import { ArrowLeft, Paperclip, iconSizeProps } from '../components/icons';
import { AppShell } from '../layout/AppShell';

interface ComposerPageProps {
  replyToThreadId?: string;
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

function apiErrorMessage(error: unknown, fallback: string) {
  if (!(error instanceof HailApiError)) return fallback;
  if (error.status === 401) return 'Your session expired. Sign in again before sending.';
  if (error.status !== 400) return `${fallback} HTTP ${error.status}.`;

  const body = error.body;
  const code = body && typeof body === 'object' && 'error' in body && typeof body.error === 'string'
    ? body.error
    : '';
  if (code === 'attachments_not_supported') {
    return 'Attachments are selected, but this server does not support sending attachments yet. Remove them and try again.';
  }
  if (code === 'invalid_send_at') return 'Choose a future send-later time.';
  if (code.includes('recipient') || code.includes('to')) return 'Check recipient addresses and try again.';
  if (code.includes('subject')) return 'Check the subject and try again.';
  if (code.includes('body')) return 'Write a message body and try again.';
  return 'Check the compose fields and try again.';
}

function composeResultMessage(response: ComposeResponse) {
  return response.status === 'pending'
    ? `Scheduled for later. Draft ${response.draft_email_id} is queued.`
    : 'Sent.';
}

export function ComposerPage({ replyToThreadId, initialTo = [], initialSubject = '', client = defaultApiClient }: ComposerPageProps) {
  const navigate = useNavigate();
  const [form, setForm] = useState<ComposerForm>({
    to: initialTo.join(', '),
    cc: '',
    bcc: '',
    subject: initialSubject,
    body: '',
    sendAt: '',
  });
  const [attachments, setAttachments] = useState<AttachmentDraft[]>([]);
  const [draftId, setDraftId] = useState<string | null>(null);
  const [dirty, setDirty] = useState(false);
  const [lastSavedAt, setLastSavedAt] = useState<Date | null>(null);
  const [successMessage, setSuccessMessage] = useState<string | null>(null);
  const [sendError, setSendError] = useState<string | null>(null);
  const [showCarbonCopyFields, setShowCarbonCopyFields] = useState(false);
  const snapshotRef = useRef('');

  const minSendAt = useMemo(() => minSendAtDateTimeLocal(), []);
  const createDraft = useCreateDraftMutation(client);
  const updateDraft = useUpdateDraftMutation(client);
  const hasUnsupportedAttachments = attachments.length > 0;
  const sendCompose = useSendComposeMutation(client, {
    onSuccess: (response) => {
      setDirty(false);
      setSendError(null);
      setSuccessMessage(composeResultMessage(response));
    },
    onError: (error) => setSendError(apiErrorMessage(error, 'Message could not be sent.')),
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
  const canSubmit = (Boolean(replyToThreadId) || draftPayload.to.length > 0)
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

  function closeComposer() {
    if (window.history.length > 1) window.history.back();
    else void navigate({ to: '/imbox' });
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

          <form onSubmit={onSubmit} className="flex min-h-0 flex-1 flex-col">
            <div className="space-y-1">
              {!replyToThreadId ? (
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
                    autoFocus
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
              ) : null}

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

              {!replyToThreadId ? (
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
              ) : null}
            </div>

            <div className="mt-8 flex min-h-[22rem] flex-1 flex-col">
              <label htmlFor="compose-body" className="sr-only">Body</label>
              <textarea
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
              <button type="submit" disabled={!canSubmit || hasUnsupportedAttachments || sendCompose.isPending} className="rounded-full bg-accent-blue px-4 py-1.5 text-xs font-semibold text-white outline-none transition hover:bg-accent-blue-hover focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent-blue disabled:cursor-not-allowed disabled:opacity-60">{sendCompose.isPending && !form.sendAt ? 'Sending…' : 'Send now'}</button>
              <button type="button" onClick={sendLater} disabled={!canSubmit || !form.sendAt || hasUnsupportedAttachments || sendCompose.isPending} className="hail-chrome text-ink-secondary outline-none hover:text-accent-blue focus-visible:rounded-md focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent-blue disabled:cursor-not-allowed disabled:opacity-50">{sendCompose.isPending && form.sendAt ? 'Scheduling…' : 'Send later'}</button>
              {!replyToThreadId ? <button type="button" onClick={saveDraft} disabled={!dirty || savingDraft || !canSaveDraft || hasUnsupportedAttachments} className="hail-chrome text-ink-tertiary outline-none hover:text-accent-blue focus-visible:rounded-md focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent-blue disabled:cursor-not-allowed disabled:opacity-50">{savingDraft ? 'Saving…' : 'Save draft'}</button> : null}
              <button type="button" onClick={closeComposer} className="ml-auto hail-chrome text-ink-tertiary outline-none hover:text-accent-red focus-visible:rounded-md focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent-blue">Discard</button>
            </div>

            <div className="mt-4 space-y-2">
              {autosaveError ? <Status kind="warn" message={apiErrorMessage(autosaveError, 'Draft autosave failed.')} /> : null}
              {sendError ? <Status kind="error" message={sendError} /> : null}
              {successMessage ? <Status kind="success" message={successMessage} /> : null}
            </div>
          </form>
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
