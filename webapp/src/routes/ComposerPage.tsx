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
import { useApiClient } from '../api/ApiClientProvider';
import {
  useCreateDraftMutation,
  useDraft,
  useSendComposeMutation,
  useThread,
  useUpdateDraftMutation,
} from '../api/query';
import { useAuth } from '../auth/AuthProvider';
import { ArrowLeft, Paperclip, iconSizeProps } from '../components/icons';
import { InlineNote } from '../components/InlineNote';
import { Alert, AlertDescription, AlertTitle } from '../components/ui/alert';
import { Badge } from '../components/ui/badge';
import { Button } from '../components/ui/button';
import { Card, CardContent } from '../components/ui/card';
import { Field, FieldGroup, FieldLabel } from '../components/ui/field';
import { Input } from '../components/ui/input';
import { Textarea } from '../components/ui/textarea';
import { useGoBack } from '../hooks/useGoBack';
import { useKeyboardShortcuts } from '../hooks/useKeyboardShortcuts';
import { AppShell } from '../layout/AppShell';
import { formatDate, formatFullDateTime } from '../lib/dates';
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
const lineInputClass = 'h-12 border-0 bg-transparent px-3 text-base shadow-none focus-visible:ring-0';

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

function escapeHtmlText(value: string) {
  return value.replace(/[&<>"']/g, (character) => {
    switch (character) {
      case '&':
        return '&amp;';
      case '<':
        return '&lt;';
      case '>':
        return '&gt;';
      case '"':
        return '&quot;';
      case "'":
        return '&#39;';
      default:
        return character;
    }
  });
}

function buildReplyQuoteHtml(message: ThreadMessage) {
  const serverQuote = (message as ThreadMessage & { reply_quote_html?: string | null }).reply_quote_html;
  if (serverQuote?.trim()) return serverQuote;

  return `<p>On ${escapeHtmlText(formatFullDateTime(message.received_at, 'an earlier message'))}, ${escapeHtmlText(formatParticipant(message.from[0]))} wrote:</p><blockquote>${message.html}</blockquote>`;
}

function prefillFromThread(
  thread: ThreadViewResponse,
  replyAll: boolean,
  currentUserEmail: string | undefined,
): ComposerForm | null {
  const messages = [...thread.messages].sort((left, right) => {
    const leftTime = Date.parse(left.received_at ?? '');
    const rightTime = Date.parse(right.received_at ?? '');
    if (Number.isNaN(leftTime) && Number.isNaN(rightTime)) return 0;
    if (Number.isNaN(leftTime)) return -1;
    if (Number.isNaN(rightTime)) return 1;
    return leftTime - rightTime;
  });
  const lastMessage = messages.at(-1);
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
    body: buildReplyQuoteHtml(lastMessage),
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

export function ComposerPage({ replyToThreadId, replyAll = false, draftId: initialDraftId, initialTo = [], initialSubject = '', client }: ComposerPageProps) {
  const contextClient = useApiClient();
  const apiClient = client ?? contextClient;
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
  const bodyRef = useRef<HTMLTextAreaElement | null>(null);
  const snapshotRef = useRef('');
  const replyPrefillKeyRef = useRef<string | null>(null);

  const minSendAt = useMemo(() => minSendAtDateTimeLocal(), []);
  const createDraft = useCreateDraftMutation(apiClient);
  const draftQuery = useDraft(initialDraftId, apiClient, { enabled: Boolean(initialDraftId) && !replyToThreadId });
  const replyThreadQuery = useThread(replyToThreadId ?? '', apiClient, { enabled: Boolean(replyToThreadId) });
  const updateDraft = useUpdateDraftMutation(apiClient);
  const hasUnsupportedAttachments = attachments.length > 0;
  const sendCompose = useSendComposeMutation(apiClient, {
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
    body_html: form.body,
    attachments: [],
  }), [form]);

  const canSaveDraft = !replyToThreadId;
  const replyPrefillLoading = Boolean(replyToThreadId) && replyThreadQuery.isLoading;
  const prefillLoading = replyPrefillLoading || draftQuery.isLoading;
  const canSubmit = !prefillLoading
    && (Boolean(replyToThreadId) || draftPayload.to.length > 0)
    && (Boolean(replyToThreadId) || form.subject.trim().length > 0)
    && form.body.trim().length > 0;
  const threadNotes = replyToThreadId && replyThreadQuery.data ? replyThreadQuery.data.notes : [];

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
      body: draftQuery.data.body_html,
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
      body_html: draftQuery.data.body_html,
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
      replyPrefillKeyRef.current = null;
      return;
    }
    if (!replyThreadQuery.data) return;

    const prefillKey = `${replyToThreadId}:${replyAll}:${user?.email ?? ''}`;
    if (replyPrefillKeyRef.current === prefillKey) return;

    setSendError(null);
    const nextForm = prefillFromThread(replyThreadQuery.data, replyAll, user?.email);
    if (nextForm) {
      setForm(nextForm);
      setShowCarbonCopyFields(nextForm.cc.length > 0);
      setDirty(false);
    }
    replyPrefillKeyRef.current = prefillKey;
  }, [replyAll, replyThreadQuery.data, replyToThreadId, user?.email]);

  useEffect(() => {
    if (replyThreadQuery.isError && replyToThreadId) {
      setSendError(composeErrorMessage(replyThreadQuery.error, 'Reply details could not be loaded.'));
    }
  }, [replyThreadQuery.error, replyThreadQuery.isError, replyToThreadId]);

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
      body_html: draftPayload.body_html,
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

  function submitComposer() {
    if (!canSubmit || sendCompose.isPending) {
      return;
    }

    send(buildComposeRequest());
  }

  function onSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    submitComposer();
  }

  function sendLater() {
    const sendAt = toFutureIsoDateTimeLocal(form.sendAt);
    if (!sendAt) {
      setSendError('Choose a future send-later time.');
      return;
    }
    send(buildComposeRequest(sendAt));
  }

  useKeyboardShortcuts({
    onSend: submitComposer,
    onEscape: closeComposer,
  });

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
      contentLayout="composer"
      reading={
        <section className="flex min-h-[calc(100vh-11rem)] flex-col" aria-labelledby="composer-title">
          <Button
            type="button"
            onClick={closeComposer}
            variant="ghost"
            size="sm"
            className="mb-6 w-fit"
          >
            <ArrowLeft data-icon="inline-start" {...iconSizeProps.sm} />
            <span>Cancel</span>
          </Button>

          <div className="sr-only">
            <h2 id="composer-title">{replyToThreadId ? 'Reply to thread' : 'Compose message'}</h2>
          </div>

          {prefillLoading ? (
            <Card className="flex min-h-[22rem] flex-1 items-center justify-center text-muted-foreground">
              <CardContent>
                {draftQuery.isLoading ? 'Loading draft…' : 'Loading reply details…'}
              </CardContent>
            </Card>
          ) : (
          <form onSubmit={onSubmit} className="flex min-h-0 flex-1 flex-col">
            <Card>
              <CardContent>
                <FieldGroup className="gap-0">
                  <Field orientation="horizontal" className="items-center gap-3 border-b border-border px-1 py-1">
                    <FieldLabel htmlFor="compose-to" className="w-16 shrink-0 pl-2 text-muted-foreground">To</FieldLabel>
                    <Input
                      id="compose-to"
                      type="text"
                      value={form.to}
                      onChange={(event) => updateField('to', event.target.value)}
                      placeholder="alice@example.com, bob@example.com"
                      className={lineInputClass}
                      autoComplete="email"
                      autoFocus={!replyToThreadId && !initialDraftId}
                    />
                    <Button
                      type="button"
                      onClick={() => setShowCarbonCopyFields((shown) => !shown)}
                      variant="ghost"
                      size="sm"
                      aria-expanded={showCarbonCopyFields}
                      aria-controls="compose-carbon-copy-fields"
                    >
                      Cc / Bcc
                    </Button>
                  </Field>

                  <div id="compose-carbon-copy-fields" className={showCarbonCopyFields ? 'grid gap-0' : 'hidden'}>
                    <Field orientation="horizontal" className="items-center gap-3 border-b border-border px-1 py-1">
                      <FieldLabel htmlFor="compose-cc" className="w-16 shrink-0 pl-2 text-muted-foreground">Cc</FieldLabel>
                      <Input
                        id="compose-cc"
                        type="text"
                        value={form.cc}
                        onChange={(event) => updateField('cc', event.target.value)}
                        className={lineInputClass}
                        autoComplete="email"
                      />
                    </Field>
                    <Field orientation="horizontal" className="items-center gap-3 border-b border-border px-1 py-1">
                      <FieldLabel htmlFor="compose-bcc" className="w-16 shrink-0 pl-2 text-muted-foreground">Bcc</FieldLabel>
                      <Input
                        id="compose-bcc"
                        type="text"
                        value={form.bcc}
                        onChange={(event) => updateField('bcc', event.target.value)}
                        className={lineInputClass}
                        autoComplete="email"
                      />
                    </Field>
                  </div>

                  <Field orientation="horizontal" className="items-center gap-3 px-1 py-1">
                    <FieldLabel htmlFor="compose-subject" className="w-16 shrink-0 pl-2 text-muted-foreground">Subject</FieldLabel>
                    <Input
                      id="compose-subject"
                      type="text"
                      value={form.subject}
                      onChange={(event) => updateField('subject', event.target.value)}
                      className={lineInputClass}
                      placeholder="Subject"
                    />
                  </Field>
                </FieldGroup>
              </CardContent>
            </Card>

            <div className="mt-6 flex min-h-[22rem] flex-1 flex-col">
              {threadNotes.length > 0 && (
                <div className="mb-4 flex flex-col gap-3">
                  <p className="text-xs font-semibold uppercase tracking-wider text-muted-foreground">Notes on this thread</p>
                  {threadNotes.map((note) => (
                    <InlineNote key={note.id} text={note.body} author="You" timestamp={formatDate(note.created_at)} />
                  ))}
                </div>
              )}
              <Field className="min-h-[22rem] flex-1">
                <FieldLabel htmlFor="compose-body" className="sr-only">Body</FieldLabel>
                <Textarea
                  ref={bodyRef}
                  id="compose-body"
                  value={form.body}
                  onChange={(event) => updateField('body', event.target.value)}
                  className="min-h-[22rem] flex-1 resize-none border-0 bg-transparent px-4 py-4 text-base leading-relaxed shadow-none focus-visible:ring-0"
                  placeholder="Write your email…"
                  autoFocus={Boolean(replyToThreadId)}
                />
              </Field>
            </div>

            <div className="mt-3 flex min-h-6 items-center justify-between gap-4">
              <div className="flex items-center gap-3 text-muted-foreground">
                <Button type="button" variant="ghost" size="sm" asChild>
                  <label htmlFor="compose-attachments" className="cursor-pointer">
                    <Paperclip data-icon="inline-start" {...iconSizeProps.sm} />
                    <span>Attachments</span>
                  </label>
                </Button>
                <Input id="compose-attachments" type="file" multiple onChange={onAttachmentsChange} className="sr-only" />
                <Field className="w-44">
                  <FieldLabel htmlFor="compose-send-at" className="sr-only">Send later</FieldLabel>
                  <Input
                    id="compose-send-at"
                    type="datetime-local"
                    value={form.sendAt}
                    min={minSendAt}
                    onChange={(event) => updateField('sendAt', event.target.value)}
                    className="h-8 text-xs text-muted-foreground"
                  />
                </Field>
              </div>
              <Badge variant="outline" className={`transition-opacity duration-500 ${savingDraft || lastSavedAt || dirty || replyToThreadId ? 'opacity-100' : 'opacity-0'}`}>
                {autosaveText}
              </Badge>
            </div>

            {attachments.length > 0 ? <AttachmentNotice attachments={attachments} /> : null}

            <div className="mt-8 flex flex-wrap items-center gap-3 border-t border-border pt-5">
              <Button type="submit" disabled={!canSubmit || hasUnsupportedAttachments || sendCompose.isPending}>{sendCompose.isPending && !form.sendAt ? 'Sending…' : 'Send now'}</Button>
              <Button type="button" onClick={sendLater} disabled={!canSubmit || !form.sendAt || hasUnsupportedAttachments || sendCompose.isPending} variant="secondary">{sendCompose.isPending && form.sendAt ? 'Scheduling…' : 'Send later'}</Button>
              {!replyToThreadId ? <Button type="button" onClick={saveDraft} disabled={!dirty || savingDraft || !canSaveDraft || hasUnsupportedAttachments} variant="outline">{savingDraft ? 'Saving…' : 'Save draft'}</Button> : null}
              <Button type="button" onClick={closeComposer} variant="ghost" className="ml-auto">Discard</Button>
            </div>

            <div className="mt-4 flex flex-col gap-2">
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
    <Alert className="mt-4">
      <AlertTitle>
        Attachments are not supported for sending or saving yet.
        <AlertDescription>
          <ul className="mt-2 flex flex-col gap-1">
            {attachments.map((attachment) => <li key={attachment.id}>{attachment.name} · {fileSizeLabel(attachment.size)} · {attachment.type}</li>)}
          </ul>
        </AlertDescription>
      </AlertTitle>
    </Alert>
  );
}

function Status({ kind, message }: { kind: 'error' | 'success' | 'warn'; message: string }) {
  return (
    <Alert variant={kind === 'error' ? 'destructive' : 'default'}>
      <AlertDescription>{message}</AlertDescription>
    </Alert>
  );
}
