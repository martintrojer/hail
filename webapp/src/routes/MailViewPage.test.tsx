import { RouterProvider } from '@tanstack/react-router';
import { cleanup, fireEvent, screen, waitFor, within } from '@testing-library/react';
import type { ReactNode } from 'react';
import { afterEach, describe, expect, it } from 'vitest';
import type {
  ImboxSectionedResponse,
  MailClassification,
  MailViewKind,
  MailViewResponse,
  ThreadVerbResponse,
} from '../api/client';
import { HailApiError } from '../api/client';
import { AuthProvider } from '../auth/AuthProvider';
import { UndoToastProvider } from '../components/UndoToastProvider';
import { router } from '../router';
import {
  createTestQueryClient,
  renderWithQueryClient,
  seedMe,
  TestHailApiClient,
} from '../test-utils';
import { MailViewPage } from './MailViewPage';

const viewRoutes: Record<MailViewKind, '/imbox' | '/feed' | '/papertrail'> = {
  imbox: '/imbox',
  feed: '/feed',
  papertrail: '/papertrail',
};

const viewTitles: Record<MailViewKind, string> = {
  imbox: 'Imbox',
  feed: 'Feed',
  papertrail: 'Paper Trail',
};

class MailViewPageTestClient extends TestHailApiClient {
  readonly calls: MailViewKind[] = [];
  readonly classifyCalls: Array<{ threadId: string; to: MailClassification }> = [];
  readonly archiveCalls: string[] = [];
  readonly trashCalls: string[] = [];
  readonly setAsideCalls: string[] = [];
  readonly markThreadCalls: Array<{ threadId: string; read: boolean }> = [];
  readonly replyLaterCalls: string[] = [];
  failingActions = new Set<string>();

  constructor(
    private readonly responses: Partial<
      Record<MailViewKind, Promise<MailViewResponse>>
    >,
  ) {
    super();
  }

  override async getImbox(): Promise<MailViewResponse> {
    this.calls.push('imbox');
    return this.responses.imbox ?? Promise.resolve(mailViewResponse('imbox'));
  }

  override async getImboxSectioned(): Promise<ImboxSectionedResponse> {
    this.calls.push('imbox');
    const response = await (this.responses.imbox ?? Promise.resolve(mailViewResponse('imbox')));

    return {
      bubbled_up: [],
      new_for_you: response.items,
      previously_seen: [],
      new_count: response.items.length,
      previously_seen_total: 0,
    };
  }

  override async getFeed(): Promise<MailViewResponse> {
    this.calls.push('feed');
    return this.responses.feed ?? Promise.resolve(mailViewResponse('feed'));
  }

  override async getPapertrail(): Promise<MailViewResponse> {
    this.calls.push('papertrail');
    return (
      this.responses.papertrail ??
      Promise.resolve(mailViewResponse('papertrail'))
    );
  }

  override async classifyThread(
    threadId: string,
    to: MailClassification,
  ): Promise<ThreadVerbResponse> {
    this.classifyCalls.push({ threadId, to });
    return this.threadVerbResponse(`classify-${to}`);
  }

  override async archiveThread(threadId: string): Promise<ThreadVerbResponse> {
    this.archiveCalls.push(threadId);
    return this.threadVerbResponse('archive');
  }

  override async trashThread(threadId: string): Promise<ThreadVerbResponse> {
    this.trashCalls.push(threadId);
    return this.threadVerbResponse('trash');
  }

  override async setAsideThread(threadId: string): Promise<ThreadVerbResponse> {
    this.setAsideCalls.push(threadId);
    return this.threadVerbResponse('set-aside');
  }

  override async markThread(threadId: string, read: boolean): Promise<void> {
    this.markThreadCalls.push({ threadId, read });
  }

  override async replyLaterThread(
    threadId: string,
  ): Promise<ThreadVerbResponse> {
    this.replyLaterCalls.push(threadId);
    return this.threadVerbResponse('reply-later');
  }

  private threadVerbResponse(action: string): ThreadVerbResponse {
    if (this.failingActions.has(action)) {
      throw new HailApiError(500, { error: 'boom' }, response(500));
    }

    return {
      undo: {
        id: `undo-${action}`,
        action: 'thread.stack',
        expires_at: '2026-05-23T13:00:00Z',
      },
    };
  }
}

let currentTestBody: ReactNode = null;
let restoreMailViewRoute: (() => void) | null = null;

function restoreRoute() {
  restoreMailViewRoute?.();
  restoreMailViewRoute = null;
}

afterEach(() => {
  currentTestBody = null;
  restoreRoute();
  window.history.pushState({}, '', '/');
  cleanup();
});

function TestBody() {
  return currentTestBody;
}

function installTestRouteComponent(view: MailViewKind) {
  const matchRoute = router.routesByPath[viewRoutes[view]];
  const previousComponent = matchRoute.options.component;
  const previousBeforeLoad = matchRoute.options.beforeLoad;
  matchRoute.options.component = TestBody;
  matchRoute.options.beforeLoad = undefined;
  restoreMailViewRoute = () => {
    matchRoute.options.component = previousComponent;
    matchRoute.options.beforeLoad = previousBeforeLoad;
  };
}

function renderMailView(
  view: MailViewKind,
  client = new MailViewPageTestClient({
    [view]: Promise.resolve(mailViewResponse(view)),
  }),
) {
  const queryClient = createTestQueryClient();

  seedMe(queryClient);

  currentTestBody = (
    <AuthProvider>
      <UndoToastProvider>
        <MailViewPage
          view={view}
          title={viewTitles[view]}
          description={`${viewTitles[view]} description`}
          client={client}
        />
      </UndoToastProvider>
    </AuthProvider>
  );
  installTestRouteComponent(view);
  window.history.pushState({}, '', viewRoutes[view]);

  renderWithQueryClient(<RouterProvider router={router} />, queryClient);

  return client;
}

function mailItem(
  classification: MailViewKind,
  overrides: Partial<MailViewResponse['items'][number]> = {},
): MailViewResponse['items'][number] {
  return {
    thread_id: 'thread-1',
    email_id: 'email-1',
    from: 'Alice Sender',
    to: ['recipient@example.com'],
    cc: [],
    bcc: [],
    subject: 'Quarterly update',
    preview: 'The latest notes from Alice.',
    received_at: '2026-05-23T12:00:00Z',
    unread: true,
    classification,
    has_notes: false,
    labels: [],
    ...overrides,
  };
}

function mailViewResponse(
  classification: MailViewKind,
  items: MailViewResponse['items'] = [mailItem(classification)],
): MailViewResponse {
  return { items, next_cursor: null };
}

function response(status: number, body: unknown = {}) {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

describe('MailViewPage', () => {
  it('renders Imbox loading, error, empty, and result states', async () => {
    const pendingClient = renderMailView(
      'imbox',
      new MailViewPageTestClient({ imbox: new Promise(() => undefined) }),
    );

    expect(
      await screen.findByLabelText('Loading Imbox mail'),
    ).toBeInTheDocument();
    expect(pendingClient.calls).toEqual(['imbox']);
    cleanup();
    restoreRoute();

    renderMailView(
      'imbox',
      new MailViewPageTestClient({
        imbox: Promise.reject(new HailApiError(401, {}, response(401))),
      }),
    );
    expect(
      await screen.findByText('Something went wrong.'),
    ).toBeInTheDocument();
    expect(
      screen.getByText(
        'Your session expired. Sign in again to refresh this view.',
      ),
    ).toBeInTheDocument();
    cleanup();
    restoreRoute();

    renderMailView(
      'imbox',
      new MailViewPageTestClient({
        imbox: Promise.resolve(mailViewResponse('imbox', [])),
      }),
    );
    expect(
      await screen.findByText("You're all caught up."),
    ).toBeInTheDocument();
    expect(screen.getByText('New mail will appear here.')).toBeInTheDocument();
    cleanup();
    restoreRoute();

    const client = renderMailView(
      'imbox',
      new MailViewPageTestClient({
        imbox: Promise.resolve(
          mailViewResponse('imbox', [
            mailItem('imbox', {
              thread_id: 'thread/needs encoding',
              email_id: 'email-imbox',
              from: 'Important Person',
              subject: 'Read this first',
              preview: 'A direct note for the Imbox.',
              unread: true,
              has_notes: true,
              labels: [
                {
                  id: 12,
                  name: 'Work/Receipts',
                  leaf_name: 'Receipts',
                  path_segments: ['Work', 'Receipts'],
                  source: 'gmail',
                  color: null,
                  thread_count: 8,
                },
              ],
            }),
          ]),
        ),
      }),
    );

    const link = await screen.findByRole('link', {
      name: 'Open Read this first from Important Person',
    });
    expect(client.calls).toEqual(['imbox']);
    expect(link).toHaveAttribute('href', '/thread/thread%2Fneeds%20encoding');
    expect(within(link).getByText('Imbox')).toBeInTheDocument();
    expect(within(link).getByText('Unread')).toBeInTheDocument();
    expect(screen.getByLabelText('Unread thread')).toBeInTheDocument();
    expect(within(link).getByLabelText('Thread has notes')).toBeInTheDocument();
    expect(within(link).getByText('Receipts')).toHaveAttribute('title', 'Work/Receipts');
    expect(within(link).getByLabelText('Label Work/Receipts')).toBeInTheDocument();
    expect(
      within(link).getByText('A direct note for the Imbox.'),
    ).toBeInTheDocument();
  });

  it.each([
    ['feed', 'Feed'],
    ['papertrail', 'Paper Trail'],
  ] as const)(
    'renders %s loading state with the matching API hook',
    async (view, title) => {
      const client = renderMailView(
        view,
        new MailViewPageTestClient({ [view]: new Promise(() => undefined) }),
      );

      expect(
        await screen.findByLabelText(`Loading ${title} mail`),
      ).toBeInTheDocument();
      expect(client.calls).toEqual([view]);
    },
  );

  it('uses the Feed hook and renders feed-specific result details', async () => {
    const client = renderMailView(
      'feed',
      new MailViewPageTestClient({
        feed: Promise.resolve(
          mailViewResponse('feed', [
            mailItem('feed', {
              thread_id: 'feed-thread',
              email_id: 'email-feed',
              from: 'Newsletter',
              subject: 'Weekly links',
              preview: 'Links worth reading this weekend.',
              unread: true,
            }),
          ]),
        ),
      }),
    );

    const link = await screen.findByRole('link', {
      name: 'Open Weekly links from Newsletter',
    });
    expect(client.calls).toEqual(['feed']);
    expect(link).toHaveAttribute('href', '/thread/feed-thread');
    expect(within(link).getByText('Feed')).toBeInTheDocument();
    expect(within(link).getByText('New')).toBeInTheDocument();
    expect(screen.getByLabelText('Unread thread')).toBeInTheDocument();
    expect(
      within(link).getByText('Links worth reading this weekend.'),
    ).toBeInTheDocument();
  });

  it('uses the Paper Trail hook and renders paper trail-specific result details', async () => {
    const client = renderMailView(
      'papertrail',
      new MailViewPageTestClient({
        papertrail: Promise.resolve(
          mailViewResponse('papertrail', [
            mailItem('papertrail', {
              thread_id: 'receipt id/2026',
              email_id: 'email-papertrail',
              from: 'Shop Example',
              subject: 'Your receipt',
              preview: 'Order #123 was paid.',
              unread: false,
            }),
          ]),
        ),
      }),
    );

    const link = await screen.findByRole('link', {
      name: 'Open Your receipt from Shop Example',
    });
    expect(client.calls).toEqual(['papertrail']);
    expect(link).toHaveAttribute('href', '/thread/receipt%20id%2F2026');
    expect(within(link).getByText('Paper Trail')).toBeInTheDocument();
    expect(within(link).getByLabelText('Read thread')).toBeInTheDocument();
    expect(within(link).queryByText('Unread')).not.toBeInTheDocument();
    expect(within(link).queryByText('New')).not.toBeInTheDocument();
    expect(
      within(link).queryByText('Order #123 was paid.'),
    ).not.toBeInTheDocument();
  });

  it('selects rows from the avatar control and runs batch actions', async () => {
    const client = renderMailView(
      'imbox',
      new MailViewPageTestClient({
        imbox: Promise.resolve(
          mailViewResponse('imbox', [
            mailItem('imbox', {
              thread_id: 'thread-one',
              email_id: 'email-one',
              from: 'Alice Sender',
              subject: 'First thread',
            }),
            mailItem('imbox', {
              thread_id: 'thread-two',
              email_id: 'email-two',
              from: 'Bob Sender',
              subject: 'Second thread',
            }),
          ]),
        ),
      }),
    );

    const firstLink = await screen.findByRole('link', {
      name: 'Open First thread from Alice Sender',
    });
    const secondLink = screen.getByRole('link', {
      name: 'Open Second thread from Bob Sender',
    });

    fireEvent.click(within(firstLink).getByRole('checkbox', { name: 'Select Alice Sender' }));
    fireEvent.click(within(secondLink).getByRole('checkbox', { name: 'Select Bob Sender' }));

    expect(screen.getByText('2 selected')).toBeInTheDocument();
    expect(
      within(firstLink).getByRole('checkbox', { name: 'Deselect Alice Sender' }),
    ).toHaveAttribute('aria-checked', 'true');

    fireEvent.click(screen.getAllByRole('button', { name: 'Set Aside' })[0]);

    await waitFor(() => expect(client.setAsideCalls).toEqual(['thread-one', 'thread-two']));
    expect(screen.queryByText('2 selected')).not.toBeInTheDocument();
    expect(screen.getByText('2 threads added to Set Aside.')).toBeInTheDocument();
  });

  it('powers through Imbox threads with thread actions and advances through the batch', async () => {
    const client = renderMailView(
      'imbox',
      new MailViewPageTestClient({
        imbox: Promise.resolve(
          mailViewResponse('imbox', [
            mailItem('imbox', {
              thread_id: 'thread-keep',
              from: 'Alice',
              subject: 'Keep me here',
              preview: 'This one should stay in the Imbox.',
            }),
            mailItem('imbox', {
              thread_id: 'thread-feed',
              from: 'Feed Sender',
              subject: 'Move me out',
              preview: 'This belongs in the Feed.',
            }),
            mailItem('imbox', {
              thread_id: 'thread-aside',
              from: 'Later Sender',
              subject: 'Handle later',
              preview: 'Set this aside for later.',
            }),
          ]),
        ),
      }),
    );

    fireEvent.click(await screen.findByRole('button', { name: /Power through/ }));

    expect(await screen.findByText('1 of 3')).toBeInTheDocument();
    expect(screen.getByText('Keep me here')).toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: 'Keep in Imbox' }));
    await waitFor(() =>
      expect(client.markThreadCalls).toEqual([
        { threadId: 'thread-keep', read: true },
      ]),
    );
    expect(await screen.findByText('Move me out')).toBeInTheDocument();
    expect(screen.getByText('2 of 3')).toBeInTheDocument();
    expect(screen.getByText('Kept thread in Imbox.')).toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: 'Move to Feed' }));
    await waitFor(() =>
      expect(client.classifyCalls).toContainEqual({
        threadId: 'thread-feed',
        to: 'feed',
      }),
    );
    expect(await screen.findByText('Handle later')).toBeInTheDocument();
    expect(screen.getByText('Moved thread to Feed.')).toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: 'Set Aside' }));
    await waitFor(() => expect(client.setAsideCalls).toEqual(['thread-aside']));
    expect(await screen.findByText('All done!')).toBeInTheDocument();
    expect(screen.getByText('Thread added to Set Aside.')).toBeInTheDocument();
  });

  it('shows power through action errors without advancing the current thread', async () => {
    const client = new MailViewPageTestClient({
      imbox: Promise.resolve(
        mailViewResponse('imbox', [
          mailItem('imbox', {
            thread_id: 'thread-fails',
            subject: 'Stays visible',
          }),
        ]),
      ),
    });
    client.failingActions.add('trash');
    renderMailView('imbox', client);

    fireEvent.click(await screen.findByRole('button', { name: /Power through/ }));
    fireEvent.click(await screen.findByRole('button', { name: 'Trash' }));

    await waitFor(() => expect(client.trashCalls).toEqual(['thread-fails']));
    expect(
      await screen.findByText('Thread action failed with HTTP 500.'),
    ).toBeInTheDocument();
    expect(screen.getByText('Stays visible')).toBeInTheDocument();
    expect(screen.queryByText('All done!')).not.toBeInTheDocument();
  });

  it('leaves power through and returns to the Imbox list when done is clicked', async () => {
    renderMailView(
      'imbox',
      new MailViewPageTestClient({
        imbox: Promise.resolve(
          mailViewResponse('imbox', [
            mailItem('imbox', {
              thread_id: 'thread-list',
              subject: 'Back to list',
            }),
          ]),
        ),
      }),
    );

    fireEvent.click(await screen.findByRole('button', { name: /Power through/ }));
    expect(await screen.findByText('Back to list')).toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: 'Exit' }));

    expect(screen.queryByText('1 of 1')).not.toBeInTheDocument();
    expect(
      screen.getByRole('link', { name: 'Open Back to list from Alice Sender' }),
    ).toBeInTheDocument();
  });

  it.each([
    [
      'feed',
      'Nothing in The Feed yet.',
      'Newsletters and notifications will show up here.',
    ],
    ['papertrail', 'No receipts yet.', 'Transactional mail will land here.'],
  ] as const)(
    'renders %s error and empty states',
    async (view, emptyTitle, emptyBody) => {
      renderMailView(
        view,
        new MailViewPageTestClient({
          [view]: Promise.reject(
            new HailApiError(503, undefined, response(503)),
          ),
        } as Partial<Record<MailViewKind, Promise<MailViewResponse>>>),
      );
      expect(
        await screen.findByText('Something went wrong.'),
      ).toBeInTheDocument();
      expect(
        screen.getByText('Mail view failed with HTTP 503.'),
      ).toBeInTheDocument();
      cleanup();
      restoreRoute();

      renderMailView(
        view,
        new MailViewPageTestClient({
          [view]: Promise.resolve(mailViewResponse(view, [])),
        } as Partial<Record<MailViewKind, Promise<MailViewResponse>>>),
      );
      expect(await screen.findByText(emptyTitle)).toBeInTheDocument();
      expect(screen.getByText(emptyBody)).toBeInTheDocument();
    },
  );
});
