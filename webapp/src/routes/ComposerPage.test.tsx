import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { RouterProvider } from '@tanstack/react-router';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import type { ReactNode } from 'react';
import { afterEach, describe, expect, it } from 'vitest';
import type {
  ComposeRequest,
  ComposeResponse,
  DraftRequest,
  DraftResponse,
  UserEnvelope,
} from '../api/client';
import { HailApiClient } from '../api/client';
import { queryKeys } from '../api/queryKeys';
import { AuthProvider } from '../auth/AuthProvider';
import { UndoToastProvider } from '../components/UndoToastProvider';
import { router } from '../router';
import { ComposerPage } from './ComposerPage';

class ComposerPageTestClient extends HailApiClient {
  readonly sendComposeCalls: ComposeRequest[] = [];
  readonly createDraftCalls: DraftRequest[] = [];

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

  override async createDraft(body: DraftRequest): Promise<DraftResponse> {
    this.createDraftCalls.push(body);
    return {
      draft_id: 'draft-1',
      updated_at: '2026-05-23T12:00:00Z',
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

function renderComposer(client = new ComposerPageTestClient()) {
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
        <ComposerPage client={client} />
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
    target: { value: 'alice@example.com' },
  });
  fireEvent.change(screen.getByLabelText('Subject'), {
    target: { value: 'Quarterly report' },
  });
  fireEvent.change(screen.getByLabelText('Body'), {
    target: { value: 'Report attached.' },
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
      to: ['alice@example.com'],
      subject: 'Quarterly report',
      body_markdown: 'Report attached.',
      attachments: [],
    });
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
  });
});
