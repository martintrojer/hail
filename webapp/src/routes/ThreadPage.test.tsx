import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { RouterProvider } from '@tanstack/react-router';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import type { ReactNode } from 'react';
import { afterEach, describe, expect, it } from 'vitest';
import type {
  BubbleUpRequest,
  BubbleUpResponse,
  CreateThreadNoteRequest,
  ContactResponse,
  ThreadVerbResponse,
  ThreadViewResponse,
  UserEnvelope,
} from '../api/client';
import { HailApiClient } from '../api/client';
import { queryKeys } from '../api/queryKeys';
import { AuthProvider } from '../auth/AuthProvider';
import { UndoToastProvider } from '../components/UndoToastProvider';
import { router } from '../router';
import { ThreadPage } from './ThreadPage';

class ThreadPageTestClient extends HailApiClient {
  readonly setAsideCalls: string[] = [];
  readonly replyLaterCalls: string[] = [];
  readonly createdNotes: Array<{ threadId: string; request: CreateThreadNoteRequest }> = [];
  readonly bubbleUpCalls: Array<{ threadId: string; request: BubbleUpRequest }> = [];

  constructor(private readonly thread: ThreadViewResponse) {
    super({ baseUrl: 'http://localhost' });
  }

  override async me(): Promise<UserEnvelope> {
    return {
      user: {
        id: 1,
        email: 'reader@example.com',
        display_name: 'Reader',
        is_admin: false,
      },
    };
  }

  override async getThread(): Promise<ThreadViewResponse> {
    return this.thread;
  }

  override async getContact(address: string): Promise<ContactResponse> {
    return {
      address,
      note: null,
      threads: [],
    };
  }

  override async createThreadNote(
    threadId: string,
    request: CreateThreadNoteRequest,
  ) {
    this.createdNotes.push({ threadId, request });
    return {
      id: 99,
      email_id: request.email_id,
      body: request.body,
      created_at: '2026-05-23T13:00:00Z',
    };
  }

  override async setAsideThread(threadId: string): Promise<ThreadVerbResponse> {
    this.setAsideCalls.push(threadId);
    return {
      undo: {
        id: 'undo-set-aside',
        action: 'thread.stack',
        expires_at: '2026-05-23T13:00:00Z',
      },
    };
  }

  override async replyLaterThread(threadId: string): Promise<ThreadVerbResponse> {
    this.replyLaterCalls.push(threadId);
    return {
      undo: {
        id: 'undo-reply-later',
        action: 'thread.stack',
        expires_at: '2026-05-23T13:00:00Z',
      },
    };
  }

  override async bubbleUpThread(
    threadId: string,
    request: BubbleUpRequest,
  ): Promise<BubbleUpResponse> {
    this.bubbleUpCalls.push({ threadId, request });
    return {
      bubble_id: 1,
      surface_at: request.at,
    };
  }
}

let currentTestBody: ReactNode = null;
let restoreThreadRoute: (() => void) | null = null;

afterEach(() => {
  currentTestBody = null;
  restoreThreadRoute?.();
  restoreThreadRoute = null;
  window.history.pushState({}, '', '/');
  cleanup();
});

function TestBody() {
  return currentTestBody;
}

function installTestRouteComponent() {
  const matchRoute = router.routesByPath['/thread/$threadId'];
  const previousComponent = matchRoute.options.component;
  const previousBeforeLoad = matchRoute.options.beforeLoad;
  matchRoute.options.component = TestBody;
  matchRoute.options.beforeLoad = undefined;
  restoreThreadRoute = () => {
    matchRoute.options.component = previousComponent;
    matchRoute.options.beforeLoad = previousBeforeLoad;
  };
}

function renderThread(thread: ThreadViewResponse) {
  const queryClient = new QueryClient({
    defaultOptions: {
      queries: { retry: false },
      mutations: { retry: false },
    },
  });
  const client = new ThreadPageTestClient(thread);

  queryClient.setQueryData(queryKeys.me(), {
    user: {
      id: 1,
      email: 'reader@example.com',
      display_name: 'Reader',
      is_admin: false,
    },
  } satisfies UserEnvelope);
  queryClient.setQueryData(queryKeys.thread(thread.thread_id), thread);

  currentTestBody = (
    <AuthProvider>
      <UndoToastProvider>
        <ThreadPage threadId={thread.thread_id} client={client} />
      </UndoToastProvider>
    </AuthProvider>
  );
  installTestRouteComponent();
  window.history.pushState({}, '', `/thread/${thread.thread_id}`);

  return {
    ...render(
      <QueryClientProvider client={queryClient}>
        <RouterProvider router={router} />
      </QueryClientProvider>,
    ),
    client,
    queryClient,
  };
}

function sampleThread(
  overrides: Partial<ThreadViewResponse> = {},
): ThreadViewResponse {
  return {
    thread_id: 'thread-1',
    subject: 'Receipt',
    participants: [{ name: 'Alice Sender', email: 'alice@example.com' }],
    messages: [
      {
        email_id: 'message-html',
        from: [{ name: 'Alice Sender', email: 'alice@example.com' }],
        to: [{ name: 'Reader', email: 'reader@example.com' }],
        received_at: '2026-05-23T12:00:00Z',
        html: '<p><strong>Sanitized receipt</strong> ready.</p>',
        preview: 'Sanitized receipt ready.',
        blocked_trackers: [
          {
            src: 'https://tracker.example/pixel.gif',
            reason: '1x1 tracking pixel removed',
          },
        ],
      },
      {
        email_id: 'message-plain',
        from: [],
        to: [],
        received_at: null,
        html: '   ',
        preview: 'Plaintext fallback line one.\nPlaintext fallback line two.',
        blocked_trackers: [],
      },
    ],
    notes: [],
    ...overrides,
  };
}

describe('ThreadPage', () => {
  it('renders sanitized HTML at the trust boundary and shows blocked trackers', async () => {
    const { container } = renderThread(sampleThread());

    expect(await screen.findByRole('heading', { name: 'Receipt' })).toBeInTheDocument();
    expect(screen.getByText('1 tracker blocked')).toHaveAttribute(
      'title',
      '1x1 tracking pixel removed',
    );
    expect(screen.getByText('Sanitized receipt')).toBeInTheDocument();

    const sanitizedHtmlBoundary = container.querySelector('article div.mt-5');
    expect(sanitizedHtmlBoundary?.innerHTML).toBe(
      '<p><strong>Sanitized receipt</strong> ready.</p>',
    );
  });

  it('renders plaintext fallback content when no sanitized HTML is available', async () => {
    renderThread(sampleThread());

    expect(await screen.findByText(/Plaintext fallback line one\./)).toBeInTheDocument();
    expect(screen.getByText(/Plaintext fallback line two\./)).toBeInTheDocument();
  });

  it('opens only one per-message action popup from the subtle dots buttons', async () => {
    renderThread(sampleThread());

    const actionButtons = await screen.findAllByRole('button', {
      name: 'Message actions',
    });
    expect(actionButtons).toHaveLength(2);

    fireEvent.click(actionButtons[0]);
    expect(screen.getAllByRole('menu', { name: 'Message actions' })).toHaveLength(1);

    fireEvent.click(actionButtons[1]);
    expect(screen.getAllByRole('menu', { name: 'Message actions' })).toHaveLength(1);

    fireEvent.click(actionButtons[1]);
    expect(
      screen.queryByRole('menu', { name: 'Message actions' }),
    ).not.toBeInTheDocument();
  });

  it('renders persisted notes from the thread response and saves new notes via API', async () => {
    const { client } = renderThread(
      sampleThread({
        notes: [
          {
            id: 7,
            email_id: 'message-html',
            body: 'Check expense category.',
            created_at: '2026-05-23T12:30:00Z',
          },
        ],
      }),
    );

    expect(await screen.findByText('Check expense category.')).toBeInTheDocument();

    const actionButtons = await screen.findAllByRole('button', {
      name: 'Message actions',
    });
    fireEvent.click(actionButtons[1]);
    fireEvent.click(screen.getByRole('button', { name: 'Add a Note' }));
    fireEvent.change(screen.getByLabelText('Note text'), {
      target: { value: 'Follow up on plain message.' },
    });
    fireEvent.click(screen.getByRole('button', { name: 'Save' }));

    await waitFor(() => {
      expect(client.createdNotes).toEqual([
        {
          threadId: 'thread-1',
          request: {
            email_id: 'message-plain',
            body: 'Follow up on plain message.',
          },
        },
      ]);
    });
    expect(await screen.findByText('Follow up on plain message.')).toBeInTheDocument();
  });

  it('does not crash on empty participants or messages', async () => {
    renderThread(sampleThread({ participants: [], messages: [] }));

    expect(await screen.findByText('0 messages with Unknown')).toBeInTheDocument();
    expect(screen.getByText('No messages in this thread')).toBeInTheDocument();
  });

  // TODO: re-add set-aside/reply-later/bubble-up tests once they're
  // accessible through the per-message popup instead of inline controls.
});
