import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { RouterProvider } from '@tanstack/react-router';
import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import type { ReactNode } from 'react';
import { afterEach, describe, expect, it } from 'vitest';
import type {
  ComposeRequest,
  ComposeResponse,
  DraftRequest,
  DraftResponse,
  ReplyRequest,
  UserEnvelope,
} from '../api/client';
import { HailApiClient, HailApiError } from '../api/client';
import { queryKeys } from '../api/queryKeys';
import { AuthProvider } from '../auth/AuthProvider';
import { UndoToastProvider } from '../components/UndoToastProvider';
import { router } from '../router';
import { ComposerPage } from './ComposerPage';

class ComposerPageTestClient extends HailApiClient {
  readonly sendComposeCalls: ComposeRequest[] = [];
  readonly sendReplyCalls: Array<{ threadId: string; body: ReplyRequest }> = [];
  readonly createDraftCalls: DraftRequest[] = [];
  readonly updateDraftCalls: Array<{ draftId: string; body: DraftRequest }> = [];
  sendComposeError: unknown;
  sendReplyError: unknown;
  createDraftError: unknown;
  updateDraftError: unknown;

  constructor() {
    super({ baseUrl: 'http://localhost' });
  }

  override async me(): Promise<UserEnvelope> {
    return {
      user: {
        id: 1,
        email: 'composer@example.com',
        display_name: 'Composer',
        is_admin: false,
      },
    };
  }

  override async sendCompose(body: ComposeRequest): Promise<ComposeResponse> {
    this.sendComposeCalls.push(body);
    if (this.sendComposeError) throw this.sendComposeError;
    if (body.send_at) {
      return {
        status: 'pending',
        scheduled_send_id: 1,
        draft_email_id: 'draft-1',
      };
    }
    return {
      status: 'sent',
      email_id: 'email-1',
      submission_id: 'submission-1',
    };
  }

  override async sendReply(threadId: string, body: ReplyRequest): Promise<ComposeResponse> {
    this.sendReplyCalls.push({ threadId, body });
    if (this.sendReplyError) throw this.sendReplyError;
    if (body.send_at) {
      return {
        status: 'pending',
        scheduled_send_id: 2,
        draft_email_id: 'reply-draft-1',
      };
    }
    return {
      status: 'sent',
      email_id: 'reply-email-1',
      submission_id: 'reply-submission-1',
    };
  }

  override async createDraft(body: DraftRequest): Promise<DraftResponse> {
    this.createDraftCalls.push(body);
    if (this.createDraftError) throw this.createDraftError;
    return {
      draft_id: 'draft-1',
      updated_at: '2026-05-23T12:00:00Z',
    };
  }

  override async updateDraft(draftId: string, body: DraftRequest): Promise<DraftResponse> {
    this.updateDraftCalls.push({ draftId, body });
    if (this.updateDraftError) throw this.updateDraftError;
    return {
      draft_id: draftId,
      updated_at: '2026-05-23T12:05:00Z',
    };
  }
}

let currentTestBody: ReactNode = null;
let restoreComposeRoute: (() => void) | null = null;

afterEach(() => {
  currentTestBody = null;
  restoreComposeRoute?.();
  restoreComposeRoute = null;
  window.history.pushState({}, '', '/');
  cleanup();
});

function TestBody() {
  return currentTestBody;
}

function installTestRouteComponent() {
  const matchRoute = router.routesByPath['/compose'];
  const previousComponent = matchRoute.options.component;
  const previousBeforeLoad = matchRoute.options.beforeLoad;
  matchRoute.options.component = TestBody;
  matchRoute.options.beforeLoad = undefined;
  restoreComposeRoute = () => {
    matchRoute.options.component = previousComponent;
    matchRoute.options.beforeLoad = previousBeforeLoad;
  };
}

interface RenderComposerOptions {
  client?: ComposerPageTestClient;
  replyToThreadId?: string;
  initialTo?: string[];
  initialSubject?: string;
}

function renderComposer({
  client = new ComposerPageTestClient(),
  replyToThreadId,
  initialTo,
  initialSubject,
}: RenderComposerOptions = {}) {
  const queryClient = new QueryClient({
    defaultOptions: {
      queries: { retry: false },
      mutations: { retry: false },
    },
  });

  queryClient.setQueryData(queryKeys.me(), {
    user: {
      id: 1,
      email: 'composer@example.com',
      display_name: 'Composer',
      is_admin: false,
    },
  } satisfies UserEnvelope);

  currentTestBody = (
    <AuthProvider>
      <UndoToastProvider>
        <ComposerPage
          client={client}
          replyToThreadId={replyToThreadId}
          initialTo={initialTo}
          initialSubject={initialSubject}
        />
      </UndoToastProvider>
    </AuthProvider>
  );
  installTestRouteComponent();
  window.history.pushState({}, '', '/compose');

  render(
    <QueryClientProvider client={queryClient}>
      <RouterProvider router={router} />
    </QueryClientProvider>,
  );

  return client;
}

async function fillSendableFields() {
  fireEvent.change(await screen.findByLabelText('To'), {
    target: { value: 'alice@example.com; bob@example.com' },
  });
  fireEvent.change(screen.getByLabelText('Cc'), {
    target: { value: 'carol@example.com' },
  });
  fireEvent.change(screen.getByLabelText('Bcc'), {
    target: { value: 'dave@example.com, erin@example.com' },
  });
  fireEvent.change(screen.getByLabelText('Subject'), {
    target: { value: 'Quarterly report' },
  });
  fireEvent.change(screen.getByLabelText('Body'), {
    target: { value: 'Report attached.' },
  });
}

async function fillReplyBody() {
  fireEvent.change(await screen.findByLabelText('Body'), {
    target: { value: 'Reply from the composer.' },
  });
}

function selectAttachment() {
  const file = new File(['hello'], 'report.pdf', { type: 'application/pdf' });
  fireEvent.change(screen.getByLabelText('Attachments'), {
    target: { files: [file] },
  });
}

function dateTimeLocalValue(date: Date) {
  const offsetMs = date.getTimezoneOffset() * 60_000;
  return new Date(date.getTime() - offsetMs).toISOString().slice(0, 16);
}

function apiError(status: number, body: unknown) {
  return new HailApiError(status, body, new Response(JSON.stringify(body), { status }));
}

describe('ComposerPage', () => {
  it('blocks sending while attachments are selected', async () => {
    const client = renderComposer();
    await fillSendableFields();
    selectAttachment();

    expect(screen.getByRole('button', { name: 'Send now' })).toBeDisabled();
    expect(screen.getByText(/Attachments are not supported for sending or saving yet\./)).toBeInTheDocument();
    fireEvent.submit(screen.getByRole('button', { name: 'Send now' }).closest('form')!);

    expect(
      await screen.findByText(/Attachments are selected, but sending and saving attachments is not supported yet\./),
    ).toBeInTheDocument();
    expect(client.sendComposeCalls).toEqual([]);
  });

  it('blocks send later while attachments are selected', async () => {
    const client = renderComposer();
    await fillSendableFields();
    fireEvent.change(screen.getByLabelText('Send later'), {
      target: { value: dateTimeLocalValue(new Date(Date.now() + 60 * 60 * 1000)) },
    });
    selectAttachment();

    expect(screen.getByRole('button', { name: 'Send later' })).toBeDisabled();
    expect(screen.getByText(/Attachments are not supported for sending or saving yet\./)).toBeInTheDocument();
    expect(client.sendComposeCalls).toEqual([]);
  });

  it('blocks saving drafts while attachments are selected', async () => {
    const client = renderComposer();
    await fillSendableFields();
    selectAttachment();

    expect(screen.getByRole('button', { name: 'Save draft' })).toBeDisabled();
    expect(screen.getByText(/Attachments are not supported for sending or saving yet\./)).toBeInTheDocument();
    expect(client.createDraftCalls).toEqual([]);
  });

  it('still sends when no attachments are selected', async () => {
    const client = renderComposer();
    await fillSendableFields();

    fireEvent.submit(screen.getByRole('button', { name: 'Send now' }).closest('form')!);

    await waitFor(() => expect(client.sendComposeCalls).toHaveLength(1));
    expect(client.sendComposeCalls[0]).toMatchObject({
      to: ['alice@example.com', 'bob@example.com'],
      cc: ['carol@example.com'],
      bcc: ['dave@example.com', 'erin@example.com'],
      subject: 'Quarterly report',
      body_markdown: 'Report attached.',
      attachments: [],
    });
    expect(await screen.findByText('Sent.')).toBeInTheDocument();
  });

  it('rejects stale send-later datetimes before calling the API', async () => {
    const client = renderComposer();
    await fillSendableFields();
    fireEvent.change(screen.getByLabelText('Send later'), {
      target: { value: '2000-01-01T00:00' },
    });

    fireEvent.click(screen.getByRole('button', { name: 'Send later' }));

    expect(await screen.findByText('Choose a future send-later time.')).toBeInTheDocument();
    expect(client.sendComposeCalls).toHaveLength(0);
  });

  it('sends an ISO send_at when the datetime is future', async () => {
    const sendAtValue = dateTimeLocalValue(new Date(Date.now() + 60 * 60 * 1000));
    const client = renderComposer();
    await fillSendableFields();
    fireEvent.change(screen.getByLabelText('Send later'), {
      target: { value: sendAtValue },
    });

    fireEvent.click(screen.getByRole('button', { name: 'Send later' }));

    await waitFor(() => expect(client.sendComposeCalls).toHaveLength(1));
    expect(client.sendComposeCalls[0]?.send_at).toBe(
      new Date(sendAtValue).toISOString(),
    );
    expect(await screen.findByText('Scheduled for later. Draft draft-1 is queued.')).toBeInTheDocument();
  });

  it('creates a draft, then updates the same draft after more edits', async () => {
    const client = renderComposer();
    await fillSendableFields();

    fireEvent.click(screen.getByRole('button', { name: 'Save draft' }));

    await waitFor(() => expect(client.createDraftCalls).toHaveLength(1));
    expect(client.createDraftCalls[0]).toEqual({
      to: ['alice@example.com', 'bob@example.com'],
      cc: ['carol@example.com'],
      bcc: ['dave@example.com', 'erin@example.com'],
      subject: 'Quarterly report',
      body_markdown: 'Report attached.',
      attachments: [],
    });
    expect(client.updateDraftCalls).toEqual([]);

    fireEvent.change(screen.getByLabelText('Body'), {
      target: { value: 'Updated draft body.' },
    });
    fireEvent.click(screen.getByRole('button', { name: 'Save draft' }));

    await waitFor(() => expect(client.updateDraftCalls).toHaveLength(1));
    expect(client.createDraftCalls).toHaveLength(1);
    expect(client.updateDraftCalls[0]).toEqual({
      draftId: 'draft-1',
      body: {
        to: ['alice@example.com', 'bob@example.com'],
        cc: ['carol@example.com'],
        bcc: ['dave@example.com', 'erin@example.com'],
        subject: 'Quarterly report',
        body_markdown: 'Updated draft body.',
        attachments: [],
      },
    });
  });

  it('uses reply mode controls and sends through the reply API', async () => {
    const client = renderComposer({
      replyToThreadId: 'thread-123',
      initialTo: ['ignored@example.com'],
      initialSubject: 'Hidden subject',
    });
    await screen.findByText('Reply to thread');

    expect(screen.queryByLabelText('To')).not.toBeInTheDocument();
    expect(screen.queryByLabelText('Subject')).not.toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Save draft' })).not.toBeInTheDocument();

    fireEvent.change(screen.getByLabelText('Cc'), {
      target: { value: 'ignored-cc@example.com' },
    });
    await fillReplyBody();
    fireEvent.click(screen.getByRole('button', { name: 'Send now' }));

    await waitFor(() => expect(client.sendReplyCalls).toHaveLength(1));
    expect(client.sendComposeCalls).toEqual([]);
    expect(client.sendReplyCalls[0]).toEqual({
      threadId: 'thread-123',
      body: {
        body_markdown: 'Reply from the composer.',
        attachments: [],
        send_at: undefined,
      },
    });
    expect(await screen.findByText('Sent.')).toBeInTheDocument();
  });

  it('shows send mutation error messages from API failures', async () => {
    const client = new ComposerPageTestClient();
    client.sendComposeError = apiError(400, { error: 'invalid_recipient' });
    renderComposer({ client });
    await fillSendableFields();

    fireEvent.click(screen.getByRole('button', { name: 'Send now' }));

    await waitFor(() => expect(client.sendComposeCalls).toHaveLength(1));
    expect(await screen.findByText('Check recipient addresses and try again.')).toBeInTheDocument();
  });

  it('shows draft mutation error messages from API failures', async () => {
    const client = new ComposerPageTestClient();
    client.createDraftError = apiError(400, { error: 'invalid_subject' });
    renderComposer({ client });
    await fillSendableFields();

    fireEvent.click(screen.getByRole('button', { name: 'Save draft' }));

    await waitFor(() => expect(client.createDraftCalls).toHaveLength(1));
    expect(await screen.findByText('Check the subject and try again.')).toBeInTheDocument();
  });

  it('keeps attachment details visible and blocks all mutations while files are selected', async () => {
    const client = renderComposer();
    await fillSendableFields();
    selectAttachment();

    const attachmentNotice = screen.getByText(/Attachments are not supported for sending or saving yet\./).closest('div');
    expect(attachmentNotice).not.toBeNull();
    expect(within(attachmentNotice!).getByText(/report\.pdf · 5 B · application\/pdf/)).toBeInTheDocument();

    expect(screen.getByRole('button', { name: 'Send now' })).toBeDisabled();
    expect(screen.getByRole('button', { name: 'Send later' })).toBeDisabled();
    expect(screen.getByRole('button', { name: 'Save draft' })).toBeDisabled();
    expect(client.sendComposeCalls).toEqual([]);
    expect(client.createDraftCalls).toEqual([]);
    expect(client.updateDraftCalls).toEqual([]);
    expect(client.sendReplyCalls).toEqual([]);
  });
});
