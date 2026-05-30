import { RouterProvider } from '@tanstack/react-router';
import { cleanup, fireEvent, screen, waitFor } from '@testing-library/react';
import type { ReactNode } from 'react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { ApiClientProvider } from '../api/ApiClientProvider';
import {
  HailApiError,
  type BubbleUpRequest,
  type BubbleUpResponse,
  type CreateThreadNoteRequest,
  type ContactResponse,
  type LabelItemResponse,
  type LabelResponse,
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
  installNoNetworkFetch,
  installNoopHistoryBack,
  installTestRoute,
  isolateAppQueryClientAuth,
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
  readonly assignLabelCalls: Array<{ threadId: string; labelId: number }> = [];
  readonly assignLabelNameCalls: Array<{ threadId: string; label_name: string }> = [];
  readonly removeLabelCalls: Array<{ threadId: string; labelId: number }> = [];
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

  override async listLabels() {
    return {
      labels: [
        ...this.thread.labels,
        labelResponse(13, 'Projects/Hail'),
        labelResponse(14, 'Personal'),
      ].filter(
        (label, index, labels) =>
          labels.findIndex((candidate) => candidate.id === label.id) === index,
      ),
    };
  }

  override async assignLabelToThread(
    threadId: string,
    labelId: number,
  ): Promise<LabelItemResponse> {
    this.assignLabelCalls.push({ threadId, labelId });
    const label = (await this.listLabels()).labels.find((candidate) => candidate.id === labelId);
    if (!label) {
      throw new HailApiError(
        404,
        { error: 'missing label' },
        new Response(JSON.stringify({ error: 'missing label' }), { status: 404 }),
      );
    }
    if (!this.thread.labels.some((existing) => existing.id === label.id)) {
      this.thread.labels = [...this.thread.labels, label];
    }
    return { label };
  }

  override async assignLabelNameToThread(
    threadId: string,
    request: { label_name: string },
  ): Promise<LabelItemResponse> {
    this.assignLabelNameCalls.push({ threadId, label_name: request.label_name });
    const label = labelResponse(99, request.label_name);
    this.thread.labels = [...this.thread.labels, label];
    return { label };
  }

  override async removeLabelFromThread(threadId: string, labelId: number): Promise<void> {
    this.removeLabelCalls.push({ threadId, labelId });
    this.thread.labels = this.thread.labels.filter((label) => label.id !== labelId);
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
let restoreAppQueryClientAuth: (() => void) | null = null;
let restoreHistoryBack: (() => void) | null = null;
let restoreNetworkFetch: (() => void) | null = null;

function restoreRouterState() {
  window.history.pushState({}, '', '/');
}

afterEach(() => {
  cleanup();
  restoreThreadRoute?.();
  restoreThreadRoute = null;
  restoreAppQueryClientAuth?.();
  restoreAppQueryClientAuth = null;
  restoreHistoryBack?.();
  restoreHistoryBack = null;
  restoreNetworkFetch?.();
  restoreNetworkFetch = null;
  currentTestBody = null;
  window.localStorage.clear();
  restoreRouterState();
});

function TestBody() {
  return currentTestBody;
}

function installTestRouteComponent() {
  restoreThreadRoute?.();
  restoreThreadRoute = installTestRoute(router, '/thread/$threadId', {
    component: TestBody,
    beforeLoad: undefined,
  });
}

function renderThread(thread: ThreadViewResponse) {
  const queryClient = createTestQueryClient();
  const client = new ThreadPageTestClient(thread);

  seedMe(queryClient, client.testUser);
  queryClient.setQueryData(queryKeys.thread(thread.thread_id), thread);

  currentTestBody = (
    <ApiClientProvider client={client}>
      <AuthProvider>
        <UndoToastProvider>
          <ThreadPage threadId={thread.thread_id} client={client} />
        </UndoToastProvider>
      </AuthProvider>
    </ApiClientProvider>
  );
  installTestRouteComponent();
  window.history.pushState({}, '', `/thread/${thread.thread_id}`);

  restoreAppQueryClientAuth?.();
  restoreAppQueryClientAuth = isolateAppQueryClientAuth(
    appQueryClient,
    client.testUser,
  );
  restoreHistoryBack?.();
  restoreHistoryBack = installNoopHistoryBack();
  restoreNetworkFetch?.();
  const noNetworkFetch = installNoNetworkFetch();
  restoreNetworkFetch = noNetworkFetch.restore;

  return {
    ...renderWithQueryClient(
      <ApiClientProvider client={client}>
        <RouterProvider router={router} />
      </ApiClientProvider>,
      queryClient,
    ),
    client,
    queryClient,
    fetchSpy: noNetworkFetch.fetchSpy,
  };
}

function labelResponse(id: number, name: string): LabelResponse {
  const pathSegments = name.split('/');
  return {
    id,
    name,
    leaf_name: pathSegments.at(-1) ?? name,
    path_segments: pathSegments,
    source: 'manual',
    color: null,
    thread_count: 0,
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
        reply_quote_html: '<p>On date, Alice Sender wrote:</p><blockquote><p><strong>Sanitized receipt</strong> ready.</p></blockquote>',
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
        reply_quote_html: '<p>On date, Unknown sender wrote:</p><blockquote></blockquote>',
        preview: 'Plaintext fallback line one.\nPlaintext fallback line two.',
        blocked_trackers: [],
      },
    ],
    notes: [],
    labels: [],
    ...overrides,
  };
}

describe('ThreadPage', () => {
  it('uses the centralized AppShell reading container instead of a route max-width wrapper', async () => {
    renderThread(sampleThread());

    expect(await screen.findByRole('button', { name: 'Back' })).toBeInTheDocument();
    const content = screen.getByTestId('app-shell-content');
    expect(content).toHaveAttribute('data-hail-content-layout', 'reading');
    expect(content).toHaveClass('max-w-3xl', 'lg:max-w-4xl', 'xl:max-w-5xl', 'min-w-0');
  });

  it('renders sanitized HTML in an isolated iframe and shows blocked trackers', async () => {
    const { container } = renderThread(sampleThread());

    expect(
      await screen.findByRole('heading', { name: 'Receipt' }),
    ).toBeInTheDocument();
    expect(screen.getByText('1 tracker blocked')).toHaveAttribute(
      'title',
      '1x1 tracking pixel removed',
    );

    const iframe = container.querySelector('iframe[title="Email body from Alice Sender"]') as HTMLIFrameElement | null;
    expect(iframe).toHaveAttribute(
      'sandbox',
      'allow-same-origin allow-popups allow-popups-to-escape-sandbox',
    );
    expect(iframe).not.toHaveAttribute('sandbox', expect.stringContaining('allow-scripts'));

    await waitFor(() => {
      expect(iframe?.contentDocument?.body.innerHTML).toBe(
        '<p><strong>Sanitized receipt</strong> ready.</p>',
      );
    });
  });



  it('renders thread labels as leaf chips with full-path titles', async () => {
    renderThread(
      sampleThread({
        labels: [
          {
            id: 12,
            name: 'Work/Receipts',
            leaf_name: 'Receipts',
            path_segments: ['Work', 'Receipts'],
            source: 'gmail',
            color: 'blue',
            thread_count: 8,
          },
        ],
      }),
    );

    expect(await screen.findByRole('heading', { name: 'Receipt' })).toBeInTheDocument();
    expect(screen.queryByRole('heading', { name: 'Manage labels' })).not.toBeInTheDocument();
    expect(screen.queryByText('Assign one or more labels to this thread.')).not.toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Manage thread labels' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Manage thread labels' })).toHaveTextContent('Labels');
    expect(screen.getByLabelText('Thread labels')).toBeInTheDocument();
    expect(screen.getByText('Receipts')).toHaveAttribute('title', 'Work/Receipts');
    expect(screen.getByLabelText('Label Work/Receipts')).toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: 'Manage thread labels' }));
    expect(await screen.findByRole('heading', { name: 'Manage labels' })).toBeInTheDocument();
    expect(screen.getByText(/Adding one label keeps/)).toBeInTheDocument();
  });

  it('adds, removes, and inline-creates labels without replacing existing labels', async () => {
    const { client, queryClient } = renderThread(
      sampleThread({
        labels: [labelResponse(12, 'Work/Receipts')],
      }),
    );
    const invalidateSpy = vi.spyOn(queryClient, 'invalidateQueries');

    expect(await screen.findByText('Receipts')).toBeInTheDocument();
    expect(screen.queryByText('Assign one or more labels to this thread.')).not.toBeInTheDocument();
    expect(screen.queryByRole('heading', { name: 'Manage labels' })).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Manage thread labels' }));
    expect(await screen.findByRole('heading', { name: 'Manage labels' })).toBeInTheDocument();
    expect(screen.getByText(/Adding one label keeps/)).toBeInTheDocument();
    expect(screen.getByText('Add or remove labels')).toBeInTheDocument();
    fireEvent.click(await screen.findByText('Hail'));

    await waitFor(() => {
      expect(client.assignLabelCalls).toEqual([{ threadId: 'thread-1', labelId: 13 }]);
    });
    await waitFor(() => {
      expect(
        queryClient
          .getQueryData<ThreadViewResponse>(queryKeys.thread('thread-1'))
          ?.labels.map((label) => label.name),
      ).toEqual(['Work/Receipts', 'Projects/Hail']);
    });

    fireEvent.click(screen.getAllByText('Receipts').at(-1)!);
    await waitFor(() => {
      expect(client.removeLabelCalls).toEqual([{ threadId: 'thread-1', labelId: 12 }]);
    });
    await waitFor(() => {
      expect(
        queryClient
          .getQueryData<ThreadViewResponse>(queryKeys.thread('thread-1'))
          ?.labels.map((label) => label.name),
      ).toEqual(['Projects/Hail']);
    });

    fireEvent.change(screen.getByPlaceholderText('Search or create label…'), {
      target: { value: 'Family/Kids' },
    });
    fireEvent.click(await screen.findByText('Create “Family/Kids”'));

    await waitFor(() => {
      expect(client.assignLabelNameCalls).toEqual([
        { threadId: 'thread-1', label_name: 'Family/Kids' },
      ]);
    });
    expect(await screen.findByText('Kids')).toBeInTheDocument();
    await waitFor(() => {
      expect(
        queryClient
          .getQueryData<ThreadViewResponse>(queryKeys.thread('thread-1'))
          ?.labels.map((label) => label.name),
      ).toEqual(['Projects/Hail', 'Family/Kids']);
    });
    expect(invalidateSpy).toHaveBeenCalledWith({ queryKey: queryKeys.thread('thread-1') });
    expect(invalidateSpy).toHaveBeenCalledWith({ queryKey: queryKeys.views() });
  });

  it('shows remote images on demand while keeping the sanitized default', async () => {
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
            reply_quote_html: '<p>On date, Alice Sender wrote:</p><blockquote><p>Logo</p></blockquote>',
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

    const firstFrame = await screen.findByTitle('Email body from Alice Sender') as HTMLIFrameElement;
    await waitFor(() => {
      expect(firstFrame.contentDocument?.querySelector('img')).toBeNull();
    });

    fireEvent.click(screen.getByRole('button', { name: 'Show remote images' }));

    await waitFor(() => {
      expect(firstFrame.contentDocument?.querySelector('img')).toHaveAttribute(
        'src',
        'https://cdn.example/logo.png',
      );
    });
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
            reply_quote_html: '<p>On date, Alice Sender wrote:</p><blockquote><p>Logo</p></blockquote>',
            preview: 'Logo',
            blocked_trackers: [],
          },
        ],
      }),
    );
    expect(await screen.findByRole('button', { name: 'Hide remote images' })).toBeInTheDocument();
    const persistedFrame = await screen.findByTitle('Email body from Alice Sender') as HTMLIFrameElement;
    await waitFor(() => {
      expect(persistedFrame.contentDocument?.querySelector('img')).toHaveAttribute(
        'src',
        'https://cdn.example/logo.png',
      );
    });
  });


  it('does not add artificial borders to email layout tables', async () => {
    renderThread(
      sampleThread({
        messages: [
          {
            email_id: 'message-table',
            from: [{ name: 'Alice Sender', email: 'alice@example.com' }],
            to: [{ name: 'Reader', email: 'reader@example.com' }],
            received_at: '2026-05-23T12:00:00Z',
            html: '<table><tbody><tr><td>Outer<table><tbody><tr><td>Inner</td></tr></tbody></table></td></tr></tbody></table>',
            html_with_remote_images: '<table><tbody><tr><td>Outer<table><tbody><tr><td>Inner</td></tr></tbody></table></td></tr></tbody></table>',
            reply_quote_html: '<p>On date, Alice Sender wrote:</p><blockquote><table><tbody><tr><td>Outer<table><tbody><tr><td>Inner</td></tr></tbody></table></td></tr></tbody></table></blockquote>',
            preview: 'Nested layout table',
            blocked_trackers: [],
          },
        ],
      }),
    );

    const iframe = await screen.findByTitle('Email body from Alice Sender') as HTMLIFrameElement;
    await waitFor(() => {
      const htmlBoundary = iframe.contentDocument?.body;
      expect(htmlBoundary?.textContent).toContain('Outer');
      expect(htmlBoundary?.querySelector('table')).toBeInTheDocument();
      expect(htmlBoundary?.innerHTML).toContain('<table>');
    });
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
    fireEvent.click(screen.getByRole('menuitem', { name: 'Feed' }));

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
    fireEvent.click(screen.getByRole('menuitem', { name: 'Paper Trail' }));
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
    const { client, queryClient, fetchSpy } = renderThread(sampleThread());
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
    expect(
      fetchSpy.mock.calls.some(([url]) =>
        String(url).endsWith('/api/auth/me'),
      ),
    ).toBe(false);
  });
});
