import { RouterProvider } from '@tanstack/react-router';
import { cleanup, fireEvent, screen, waitFor } from '@testing-library/react';
import type { ReactNode } from 'react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import {
  HailApiError,
  type BubbleUpRequest,
  type BubbleUpResponse,
  type CreateThreadNoteRequest,
  type ContactResponse,
  type MailClassification,
  type ThreadVerbResponse,
  type ThreadViewResponse,
} from '../api/client';
import { queryKeys } from '../api/queryKeys';
import { AuthProvider } from '../auth/AuthProvider';
import { UndoToastProvider } from '../components/UndoToastProvider';
import { router } from '../router';
import { queryClient as appQueryClient } from '../lib/queryClient';
import {
  createTestQueryClient,
  renderWithQueryClient,
  seedMe,
  TestHailApiClient,
} from '../test-utils';
import { ThreadPage } from './ThreadPage';

class ThreadPageTestClient extends TestHailApiClient {
  readonly setAsideCalls: string[] = [];
  readonly replyLaterCalls: string[] = [];
  readonly trashCalls: string[] = [];
  readonly archiveCalls: string[] = [];
  readonly classifyCalls: Array<{ threadId: string; to: MailClassification }> = [];
  readonly createdNotes: Array<{
    threadId: string;
    request: CreateThreadNoteRequest;
  }> = [];
  readonly bubbleUpCalls: Array<{
    threadId: string;
    request: BubbleUpRequest;
  }> = [];
  readonly markThreadCalls: Array<{ threadId: string; read: boolean }> = [];

  failingActions = new Set<string>();

  constructor(private readonly thread: ThreadViewResponse) {
    super();
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

  override async markThread(threadId: string, read: boolean): Promise<void> {
    this.markThreadCalls.push({ threadId, read });
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

  private threadVerbResponse(action: string): ThreadVerbResponse {
    if (this.failingActions.has(action)) {
      throw new HailApiError(
        500,
        { error: 'boom' },
        new Response(JSON.stringify({ error: 'boom' }), { status: 500 }),
      );
    }

    return {
      undo: {
        id: `undo-${action}`,
        action: 'thread.stack',
        expires_at: '2026-05-23T13:00:00Z',
      },
    };
  }

  override async setAsideThread(threadId: string): Promise<ThreadVerbResponse> {
    this.setAsideCalls.push(threadId);
    return this.threadVerbResponse('set-aside');
  }

  override async replyLaterThread(
    threadId: string,
  ): Promise<ThreadVerbResponse> {
    this.replyLaterCalls.push(threadId);
    return this.threadVerbResponse('reply-later');
  }

  override async trashThread(threadId: string): Promise<ThreadVerbResponse> {
    this.trashCalls.push(threadId);
    return this.threadVerbResponse('trash');
  }

  override async archiveThread(threadId: string): Promise<ThreadVerbResponse> {
    this.archiveCalls.push(threadId);
    return this.threadVerbResponse('archive');
  }

  override async classifyThread(
    threadId: string,
    to: MailClassification,
  ): Promise<ThreadVerbResponse> {
    this.classifyCalls.push({ threadId, to });
    return this.threadVerbResponse(`move-${to}`);
  }

  override async bubbleUpThread(
    threadId: string,
    request: BubbleUpRequest,
  ): Promise<BubbleUpResponse> {
    this.bubbleUpCalls.push({ threadId, request });
    if (this.failingActions.has('bubble-up')) {
      throw new HailApiError(
        500,
        { error: 'boom' },
        new Response(JSON.stringify({ error: 'boom' }), { status: 500 }),
      );
    }
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
  window.localStorage.clear();
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
  const queryClient = createTestQueryClient();
  const client = new ThreadPageTestClient(thread);

  seedMe(queryClient);
  seedMe(appQueryClient);
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
    ...renderWithQueryClient(<RouterProvider router={router} />, queryClient),
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
        html_with_remote_images: '<p><strong>Sanitized receipt</strong> ready.</p>',
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
        html_with_remote_images: '   ',
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

    expect(
      await screen.findByRole('heading', { name: 'Receipt' }),
    ).toBeInTheDocument();
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



  it('shows remote images on demand while keeping the sanitized default', async () => {
    const { container } = renderThread(
      sampleThread({
        messages: [
          {
            email_id: 'message-remote',
            from: [{ name: 'Alice Sender', email: 'alice@example.com' }],
            to: [{ name: 'Reader', email: 'reader@example.com' }],
            received_at: '2026-05-23T12:00:00Z',
            html: '<p>Logo</p>',
            html_with_remote_images: '<p>Logo</p><img src="https://cdn.example/logo.png" alt="Logo">',
            preview: 'Logo',
            blocked_trackers: [
              {
                src: 'https://cdn.example/logo.png',
                reason: 'remote image blocked by default',
              },
              {
                src: 'https://tracker.example/open.gif',
                reason: 'image dimensions are 2x2 or smaller',
              },
            ],
          },
        ],
      }),
    );

    expect(await screen.findByText(/Remote images are hidden by default\./)).toBeInTheDocument();
    expect(container.querySelector('article img')).toBeNull();

    fireEvent.click(screen.getByRole('button', { name: 'Show remote images' }));

    const image = container.querySelector('article img');
    expect(image).not.toBeNull();
    expect(image).toHaveAttribute('src', 'https://cdn.example/logo.png');
    expect(screen.getByRole('button', { name: 'Hide remote images' })).toBeInTheDocument();

    cleanup();
    renderThread(
      sampleThread({
        messages: [
          {
            email_id: 'message-remote',
            from: [{ name: 'Alice Sender', email: 'alice@example.com' }],
            to: [{ name: 'Reader', email: 'reader@example.com' }],
            received_at: '2026-05-23T12:00:00Z',
            html: '<p>Logo</p>',
            html_with_remote_images: '<p>Logo</p><img src="https://cdn.example/logo.png" alt="Logo">',
            preview: 'Logo',
            blocked_trackers: [],
          },
        ],
      }),
    );
    expect(await screen.findByRole('button', { name: 'Hide remote images' })).toBeInTheDocument();
    expect(document.querySelector('article img')).toHaveAttribute('src', 'https://cdn.example/logo.png');
  });


  it('renders plaintext fallback content when no sanitized HTML is available', async () => {
    renderThread(sampleThread());

    expect(
      await screen.findByText(/Plaintext fallback line one\./),
    ).toBeInTheDocument();
    expect(
      screen.getByText(/Plaintext fallback line two\./),
    ).toBeInTheDocument();
  });

  it('opens only one per-message action popup from the subtle dots buttons', async () => {
    renderThread(sampleThread());

    const actionButtons = await screen.findAllByRole('button', {
      name: 'Message actions',
    });
    expect(actionButtons).toHaveLength(2);

    fireEvent.click(actionButtons[0]);
    expect(
      screen.getAllByRole('menu', { name: 'Message actions' }),
    ).toHaveLength(1);

    fireEvent.click(actionButtons[1]);
    expect(
      screen.getAllByRole('menu', { name: 'Message actions' }),
    ).toHaveLength(1);

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

    expect(
      await screen.findByText('Check expense category.'),
    ).toBeInTheDocument();

    const actionButtons = await screen.findAllByRole('button', {
      name: 'Message actions',
    });
    fireEvent.click(actionButtons[1]);
    fireEvent.click(screen.getByRole('menuitem', { name: 'Add a Note' }));
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
    expect(
      await screen.findByText('Follow up on plain message.'),
    ).toBeInTheDocument();
  });

  it('does not crash on empty participants or messages', async () => {
    renderThread(sampleThread({ participants: [], messages: [] }));

    expect(
      await screen.findByText('0 messages with Unknown'),
    ).toBeInTheDocument();
    expect(screen.getByText('No messages in this thread')).toBeInTheDocument();
  });

  it('routes thread popup actions through mutations and invalidates thread/view caches', async () => {
    const { client, queryClient } = renderThread(sampleThread());
    const invalidateSpy = vi.spyOn(queryClient, 'invalidateQueries');

    const actionButtons = await screen.findAllByRole('button', {
      name: 'Message actions',
    });
    fireEvent.click(actionButtons[0]);
    fireEvent.click(screen.getByRole('menuitem', { name: 'Set Aside' }));

    await waitFor(() => {
      expect(client.setAsideCalls).toEqual(['thread-1']);
    });
    expect(invalidateSpy).toHaveBeenCalledWith({ queryKey: queryKeys.thread('thread-1') });
    expect(invalidateSpy).toHaveBeenCalledWith({ queryKey: queryKeys.views() });
    expect(await screen.findByText('Thread added to Set Aside.')).toBeInTheDocument();

    fireEvent.click(actionButtons[0]);
    fireEvent.click(screen.getByRole('button', { name: 'Feed' }));

    await waitFor(() => {
      expect(client.classifyCalls).toEqual([{ threadId: 'thread-1', to: 'feed' }]);
    });
    expect(await screen.findByText('Moved thread to Feed.')).toBeInTheDocument();
  });

  it('keeps the thread open and shows an action error when a mutation fails', async () => {
    const { client } = renderThread(sampleThread());
    client.failingActions.add('trash');

    const actionButtons = await screen.findAllByRole('button', {
      name: 'Message actions',
    });
    fireEvent.click(actionButtons[0]);
    fireEvent.click(screen.getByRole('menuitem', { name: 'Trash' }));

    expect(await screen.findByRole('alert')).toHaveTextContent(
      'Thread action failed with HTTP 500.',
    );
    expect(screen.getByRole('heading', { name: 'Receipt' })).toBeInTheDocument();
    expect(client.trashCalls).toEqual(['thread-1']);
  });

  it('routes every thread popup verb through API mutations, cache invalidation, and undo UI', async () => {
    const { client, queryClient } = renderThread(sampleThread());
    const invalidateSpy = vi.spyOn(queryClient, 'invalidateQueries');

    const actionButtons = await screen.findAllByRole('button', {
      name: 'Message actions',
    });

    fireEvent.click(actionButtons[0]);
    fireEvent.click(screen.getByRole('menuitem', { name: 'Reply Later' }));
    await waitFor(() => expect(client.replyLaterCalls).toEqual(['thread-1']));
    expect(
      await screen.findByText('Thread added to Reply Later.'),
    ).toBeInTheDocument();

    fireEvent.click(actionButtons[0]);
    fireEvent.click(screen.getByRole('menuitem', { name: 'Archive' }));
    await waitFor(() => expect(client.archiveCalls).toEqual(['thread-1']));

    fireEvent.click(actionButtons[0]);
    fireEvent.click(screen.getByRole('button', { name: 'Paper Trail' }));
    await waitFor(() => {
      expect(client.classifyCalls).toContainEqual({
        threadId: 'thread-1',
        to: 'papertrail',
      });
    });
    expect(
      await screen.findByText('Moved thread to Paper Trail.'),
    ).toBeInTheDocument();

    fireEvent.click(actionButtons[0]);
    fireEvent.click(screen.getByRole('menuitem', { name: 'Trash' }));
    await waitFor(() => expect(client.trashCalls).toEqual(['thread-1']));
    expect(await screen.findByText('Thread moved to trash.')).toBeInTheDocument();

    expect(invalidateSpy).toHaveBeenCalledWith({
      queryKey: queryKeys.thread('thread-1'),
    });
    expect(invalidateSpy).toHaveBeenCalledWith({ queryKey: queryKeys.views() });
    expect(client.markThreadCalls).toEqual([
      { threadId: 'thread-1', read: true },
    ]);
  });

  it('routes reply, reply-all, and forward popup actions to compose search params', async () => {
    renderThread(sampleThread());

    const actionButtons = await screen.findAllByRole('button', {
      name: 'Message actions',
    });

    fireEvent.click(actionButtons[0]);
    fireEvent.click(screen.getByRole('menuitem', { name: 'Reply' }));
    await waitFor(() => {
      expect(window.location.pathname).toBe('/compose');
      expect(window.location.search).toContain('replyTo=thread-1');
      expect(window.location.search).toContain('replyAll=false');
    });

    window.history.pushState({}, '', '/thread/thread-1');
    await router.invalidate();
    const firstReplyAllActionButton = await screen
      .findAllByRole('button', { name: 'Message actions' })
      .then((buttons) => buttons[0]);
    fireEvent.click(firstReplyAllActionButton);
    fireEvent.click(screen.getByRole('menuitem', { name: 'Reply All' }));
    await waitFor(() => {
      expect(window.location.pathname).toBe('/compose');
      expect(window.location.search).toContain('replyTo=thread-1');
      expect(window.location.search).toContain('replyAll=true');
    });

    window.history.pushState({}, '', '/thread/thread-1');
    await router.invalidate();
    const firstForwardActionButton = await screen
      .findAllByRole('button', { name: 'Message actions' })
      .then((buttons) => buttons[0]);
    fireEvent.click(firstForwardActionButton);
    fireEvent.click(screen.getByRole('menuitem', { name: 'Forward' }));
    await waitFor(() => {
      expect(window.location.pathname).toBe('/compose');
      expect(window.location.search).toContain('forward=message-html');
    });
  });

  it('routes bubble-up selections through the shared mutation', async () => {
    const { client, queryClient } = renderThread(sampleThread());
    const invalidateSpy = vi.spyOn(queryClient, 'invalidateQueries');

    const actionButtons = await screen.findAllByRole('button', {
      name: 'Message actions',
    });
    fireEvent.click(actionButtons[0]);
    fireEvent.click(screen.getByRole('menuitem', { name: 'Bubble Up' }));
    fireEvent.click(screen.getByRole('menuitem', { name: 'Tomorrow morning' }));

    await waitFor(() => {
      expect(client.bubbleUpCalls).toHaveLength(1);
    });
    expect(client.bubbleUpCalls[0].threadId).toBe('thread-1');
    expect(new Date(client.bubbleUpCalls[0].request.at).valueOf()).not.toBeNaN();
    expect(invalidateSpy).toHaveBeenCalledWith({ queryKey: queryKeys.thread('thread-1') });
    expect(invalidateSpy).toHaveBeenCalledWith({ queryKey: queryKeys.views() });
    expect(await screen.findByText(/Thread will bubble up at/)).toBeInTheDocument();
  });
});
