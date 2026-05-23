import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { RouterProvider } from '@tanstack/react-router';
import { cleanup, render, screen } from '@testing-library/react';
import type { ReactNode } from 'react';
import { afterEach, describe, expect, it } from 'vitest';
import type {
  ContactResponse,
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

  return render(
    <QueryClientProvider client={queryClient}>
      <RouterProvider router={router} />
    </QueryClientProvider>,
  );
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

  it('does not crash on empty participants or messages', async () => {
    renderThread(sampleThread({ participants: [], messages: [] }));

    expect(await screen.findByText('0 messages with Unknown')).toBeInTheDocument();
    expect(screen.getByText('No messages in this thread')).toBeInTheDocument();
  });
});
