import {
  useEffect,
  useMemo,
  useRef,
  useState,
  type ChangeEvent,
  type FormEvent,
  type ReactNode,
} from 'react';
import { useNavigate } from '@tanstack/react-router';
import { HailApiError, type HailApiClient, type ComposeRequest, type ComposeResponse } from '../api/client';
import {
  defaultApiClient,
  useCreateDraftMutation,
  useSendComposeMutation,
  useUpdateDraftMutation,
} from '../api/query';
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
const inputClass = 'rounded-lg border border-slate-700 bg-slate-950 px-3 py-2 text-sm text-slate-50 outline-none ring-sky-400 transition placeholder:text-slate-600 focus:border-sky-400 focus:ring-2';

function splitAddresses(value: string) {
  return value.split(/[;,]/).map((address) => address.trim()).filter(Boolean);
}

function toIsoDateTimeLocal(value: string) {
  const date = new Date(value);
  return value && !Number.isNaN(date.valueOf()) ? date.toISOString() : undefined;
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
  const snapshotRef = useRef('');

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
    if (!dirty || replyToThreadId || !canSubmit) return;
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
    const sendAt = toIsoDateTimeLocal(form.sendAt);
    if (!sendAt) {
      setSendError('Choose a valid future send-later time.');
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

  return (
    <AppShell
      title={replyToThreadId ? 'Reply' : 'Compose'}
      description="Drafts auto-save every five seconds while you write."
      reading={
        <section className="mx-auto max-w-3xl rounded-3xl border border-slate-800 bg-slate-900/80 p-6 shadow-2xl shadow-slate-950/40" aria-labelledby="composer-title">
          <div className="flex items-start justify-between gap-4">
            <div>
              <p className="text-xs font-semibold uppercase tracking-[0.3em] text-sky-300">{replyToThreadId ? 'Reply composer' : 'New message'}</p>
              <h2 id="composer-title" className="mt-2 text-2xl font-semibold text-slate-50">{replyToThreadId ? 'Reply to thread' : 'Compose message'}</h2>
            </div>
            <button type="button" onClick={closeComposer} className="rounded-lg border border-slate-700 px-3 py-1.5 text-sm font-semibold text-slate-200 transition hover:border-sky-400 hover:text-sky-100">Close</button>
          </div>

          <form onSubmit={onSubmit} className="mt-6 space-y-4">
            {!replyToThreadId ? (
              <Field id="compose-to" label="To">
                <input id="compose-to" type="text" value={form.to} onChange={(event) => updateField('to', event.target.value)} placeholder="alice@example.com, bob@example.com" className={`mt-2 w-full ${inputClass}`} autoComplete="email" autoFocus />
              </Field>
            ) : null}

            <div className="grid gap-4 md:grid-cols-2">
              <Field id="compose-cc" label="Cc">
                <input id="compose-cc" type="text" value={form.cc} onChange={(event) => updateField('cc', event.target.value)} className={`mt-2 w-full ${inputClass}`} autoComplete="email" />
              </Field>
              <Field id="compose-bcc" label="Bcc">
                <input id="compose-bcc" type="text" value={form.bcc} onChange={(event) => updateField('bcc', event.target.value)} className={`mt-2 w-full ${inputClass}`} autoComplete="email" />
              </Field>
            </div>

            {!replyToThreadId ? (
              <Field id="compose-subject" label="Subject">
                <input id="compose-subject" type="text" value={form.subject} onChange={(event) => updateField('subject', event.target.value)} className={`mt-2 w-full ${inputClass}`} placeholder="Subject" />
              </Field>
            ) : null}

            <Field id="compose-body" label="Body">
              <textarea id="compose-body" value={form.body} onChange={(event) => updateField('body', event.target.value)} className={`mt-2 min-h-72 w-full resize-y leading-6 ${inputClass}`} placeholder="Write your message…" autoFocus={Boolean(replyToThreadId)} />
            </Field>

            <div className="grid gap-4 md:grid-cols-2">
              <Field id="compose-attachments" label="Attachments">
                <input id="compose-attachments" type="file" multiple onChange={onAttachmentsChange} className="mt-2 w-full rounded-lg border border-dashed border-slate-700 bg-slate-950 px-3 py-2 text-sm text-slate-300 file:mr-3 file:rounded-md file:border-0 file:bg-slate-800 file:px-3 file:py-1.5 file:text-sm file:font-semibold file:text-slate-100 hover:border-slate-600" />
              </Field>
              <Field id="compose-send-at" label="Send later">
                <input id="compose-send-at" type="datetime-local" value={form.sendAt} onChange={(event) => updateField('sendAt', event.target.value)} className={`mt-2 w-full ${inputClass}`} />
              </Field>
            </div>

            {attachments.length > 0 ? <AttachmentNotice attachments={attachments} /> : null}

            <div className="flex flex-wrap items-center gap-3 border-t border-slate-800 pt-4">
              <button type="submit" disabled={!canSubmit || hasUnsupportedAttachments || sendCompose.isPending} className="rounded-lg bg-sky-400 px-4 py-2 text-sm font-semibold text-slate-950 transition hover:bg-sky-300 disabled:cursor-not-allowed disabled:opacity-60">{sendCompose.isPending && !form.sendAt ? 'Sending…' : 'Send now'}</button>
              <button type="button" onClick={sendLater} disabled={!canSubmit || !form.sendAt || hasUnsupportedAttachments || sendCompose.isPending} className="rounded-lg border border-slate-700 px-4 py-2 text-sm font-semibold text-slate-100 transition hover:border-sky-400 hover:text-sky-100 disabled:cursor-not-allowed disabled:opacity-60">{sendCompose.isPending && form.sendAt ? 'Scheduling…' : 'Send later'}</button>
              {!replyToThreadId ? <button type="button" onClick={saveDraft} disabled={!dirty || savingDraft || !canSubmit || hasUnsupportedAttachments} className="rounded-lg border border-slate-700 px-4 py-2 text-sm font-semibold text-slate-100 transition hover:border-slate-500 disabled:cursor-not-allowed disabled:opacity-60">{savingDraft ? 'Saving…' : 'Save draft'}</button> : null}
              <p className="text-xs text-slate-500">{replyToThreadId ? 'Replies send through the thread reply API.' : savingDraft ? 'Autosaving…' : lastSavedAt ? `Saved ${lastSavedAt.toLocaleTimeString()}` : dirty ? 'Unsaved changes' : 'No draft saved yet'}</p>
            </div>

            {autosaveError ? <Status kind="warn" message={apiErrorMessage(autosaveError, 'Draft autosave failed.')} /> : null}
            {sendError ? <Status kind="error" message={sendError} /> : null}
            {successMessage ? <Status kind="success" message={successMessage} /> : null}
          </form>
        </section>
      }
    />
  );
}

function Field({ id, label, children }: { id: string; label: string; children: ReactNode }) {
  return <label className="block text-sm font-medium text-slate-200" htmlFor={id}>{label}{children}</label>;
}

function AttachmentNotice({ attachments }: { attachments: AttachmentDraft[] }) {
  return (
    <div className="rounded-2xl border border-amber-500/30 bg-amber-500/10 p-3 text-sm text-amber-100">
      <p className="font-semibold">Attachments are not supported for sending or saving yet.</p>
      <ul className="mt-2 space-y-1 text-amber-50/90">
        {attachments.map((attachment) => <li key={attachment.id}>{attachment.name} · {fileSizeLabel(attachment.size)} · {attachment.type}</li>)}
      </ul>
    </div>
  );
}

function Status({ kind, message }: { kind: 'error' | 'success' | 'warn'; message: string }) {
  const className = kind === 'success'
    ? 'border-emerald-800 bg-emerald-950/70 text-emerald-100'
    : kind === 'warn'
      ? 'border-amber-800 bg-amber-950/70 text-amber-100'
      : 'border-red-800 bg-red-950/70 text-red-100';
  return <p className={`rounded-lg border px-3 py-2 text-sm ${className}`}>{message}</p>;
}
