import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { RouterProvider } from '@tanstack/react-router';
import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from '@testing-library/react';
import type { ReactNode } from 'react';
import { afterEach, describe, expect, it } from 'vitest';
import type {
  DeniedSendersResponse,
  ScreenerDecisionRequest,
  ScreenerDecisionResponse,
  ScreenerView,
  UserEnvelope,
} from '../api/client';
import { HailApiClient, HailApiError } from '../api/client';
import { queryKeys } from '../api/queryKeys';
import { AuthProvider } from '../auth/AuthProvider';
import { UndoToastProvider } from '../components/UndoToastProvider';
import { router } from '../router';
import { ScreenerPage } from './ScreenerPage';

class ScreenerPageTestClient extends HailApiClient {
  readonly decideScreenerCalls: ScreenerDecisionRequest[] = [];
  readonly undoDenyCalls: string[] = [];
  private viewPromise: Promise<ScreenerView>;
  private deniedPromise: Promise<DeniedSendersResponse>;
  private decisionHandler: (
    body: ScreenerDecisionRequest,
  ) => Promise<ScreenerDecisionResponse>;

  constructor({
    view = sampleScreenerView(),
    denied = sampleDeniedSenders(),
    viewPromise,
    deniedPromise,
    decisionHandler,
  }: {
    view?: ScreenerView;
    denied?: DeniedSendersResponse;
    viewPromise?: Promise<ScreenerView>;
    deniedPromise?: Promise<DeniedSendersResponse>;
    decisionHandler?: (
      body: ScreenerDecisionRequest,
    ) => Promise<ScreenerDecisionResponse>;
  } = {}) {
    super({ baseUrl: 'http://localhost' });
    this.viewPromise = viewPromise ?? Promise.resolve(view);
    this.deniedPromise = deniedPromise ?? Promise.resolve(denied);
    this.decisionHandler =
      decisionHandler ??
      ((body) =>
        Promise.resolve({
          sender: body.sender,
          decision: body.decision,
          classify_as:
            body.classify_as === 'imbox' ||
            body.classify_as === 'feed' ||
            body.classify_as === 'papertrail'
              ? body.classify_as
              : null,
        }));
  }

  override async me(): Promise<UserEnvelope> {
    return {
      user: {
        id: 1,
        email: 'screener@example.com',
        display_name: 'Screener',
        is_admin: false,
      },
    };
  }

  override async getScreenerView(): Promise<ScreenerView> {
    return this.viewPromise;
  }

  override async getDeniedSenders(): Promise<DeniedSendersResponse> {
    return this.deniedPromise;
  }

  override async decideScreener(
    body: ScreenerDecisionRequest,
  ): Promise<ScreenerDecisionResponse> {
    this.decideScreenerCalls.push(body);
    return this.decisionHandler(body);
  }

  override async undoDeny(address: string) {
    this.undoDenyCalls.push(address);
    return { status: 'undone' as const };
  }
}

let currentTestBody: ReactNode = null;
let restoreScreenerRoute: (() => void) | null = null;

function restoreRoute() {
  restoreScreenerRoute?.();
  restoreScreenerRoute = null;
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

function installTestRouteComponent() {
  const matchRoute = router.routesByPath['/screener'];
  const previousComponent = matchRoute.options.component;
  const previousBeforeLoad = matchRoute.options.beforeLoad;
  matchRoute.options.component = TestBody;
  matchRoute.options.beforeLoad = undefined;
  restoreScreenerRoute = () => {
    matchRoute.options.component = previousComponent;
    matchRoute.options.beforeLoad = previousBeforeLoad;
  };
}

function renderScreener(client = new ScreenerPageTestClient()) {
  const queryClient = new QueryClient({
    defaultOptions: {
      queries: { retry: false },
      mutations: { retry: false },
    },
  });

  queryClient.setQueryData(queryKeys.me(), {
    user: {
      id: 1,
      email: 'screener@example.com',
      display_name: 'Screener',
      is_admin: false,
    },
  } satisfies UserEnvelope);

  currentTestBody = (
    <AuthProvider>
      <UndoToastProvider>
        <ScreenerPage client={client} />
      </UndoToastProvider>
    </AuthProvider>
  );
  installTestRouteComponent();
  window.history.pushState({}, '', '/screener');

  render(
    <QueryClientProvider client={queryClient}>
      <RouterProvider router={router} />
    </QueryClientProvider>,
  );

  return client;
}

function sampleScreenerView(overrides: Partial<ScreenerView> = {}): ScreenerView {
  return {
    senders: [
      {
        sender: 'newsletter@example.com',
        first_seen_at: '2026-05-23T12:00:00Z',
        message_count: 3,
        latest_preview: {
          from: 'newsletter@example.com',
          subject: 'Newsletter dispatch',
          preview: 'Latest dispatch from the newsletter.',
          received_at: '2026-05-23T12:15:00Z',
        },
      },
    ],
    ...overrides,
  };
}

function sampleDeniedSenders(
  overrides: Partial<DeniedSendersResponse> = {},
): DeniedSendersResponse {
  return {
    denied: [
      {
        sender_address: 'blocked@example.com',
        denied_at: '2026-05-22T10:00:00Z',
      },
    ],
    ...overrides,
  };
}

function response(status: number, body: unknown = {}) {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

describe('ScreenerPage', () => {
  it('opens routing choices before approving a sender with history backfill', async () => {
    const client = renderScreener();

    fireEvent.click(await screen.findByRole('button', { name: 'Approve' }));
    expect(
      screen.getByRole('menu', { name: 'Screener routing destinations' }),
    ).toBeInTheDocument();
    expect(screen.getByRole('menuitem', { name: 'The Imbox' })).toHaveClass(
      'bg-bg-selected',
    );

    fireEvent.click(screen.getByRole('menuitem', { name: 'The Feed' }));

    await waitFor(() => expect(client.decideScreenerCalls).toHaveLength(1));
    expect(client.decideScreenerCalls[0]).toEqual({
      sender: 'newsletter@example.com',
      decision: 'approve',
      classify_as: 'feed',
      apply_to_history: true,
    });
  });

  it('denies a sender without classification and applies the decision to history', async () => {
    const client = renderScreener();

    fireEvent.click(await screen.findByRole('button', { name: 'Deny' }));

    await waitFor(() => expect(client.decideScreenerCalls).toHaveLength(1));
    expect(client.decideScreenerCalls[0]).toEqual({
      sender: 'newsletter@example.com',
      decision: 'deny',
      apply_to_history: true,
    });
  });

  it('shows denied senders after expanding and can undo a denied sender', async () => {
    const client = renderScreener();

    expect(screen.getByRole('button', { name: /Previously denied/i })).toHaveAttribute(
      'aria-expanded',
      'false',
    );
    expect(screen.queryByText('blocked@example.com')).not.toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: /Previously denied/i }));

    expect(await screen.findByText('blocked@example.com')).toBeInTheDocument();
    expect(screen.getByText(/Denied/)).toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: 'Undo' }));

    await waitFor(() => expect(client.undoDenyCalls).toEqual(['blocked@example.com']));
    expect(
      await screen.findByText('Restored blocked@example.com to the Screener.'),
    ).toBeInTheDocument();
  });

  it('shows initial pending, error, and empty states', async () => {
    const neverResolves = new Promise<ScreenerView>(() => undefined);
    const pendingClient = renderScreener(
      new ScreenerPageTestClient({ viewPromise: neverResolves }),
    );

    expect(screen.getByLabelText('Loading pending senders')).toBeInTheDocument();
    expect(pendingClient.decideScreenerCalls).toEqual([]);
    cleanup();
    restoreRoute();

    renderScreener(
      new ScreenerPageTestClient({
        viewPromise: Promise.reject(new HailApiError(401, {}, response(401))),
      }),
    );
    expect(
      await screen.findByText('Something went wrong.'),
    ).toBeInTheDocument();
    expect(
      screen.getByText('Your session expired. Sign in again to refresh the Screener.'),
    ).toBeInTheDocument();
    cleanup();
    restoreRoute();

    renderScreener(new ScreenerPageTestClient({ view: sampleScreenerView({ senders: [] }) }));
    expect(await screen.findByText('No unknown senders')).toBeInTheDocument();
  });

  it('disables card controls while a decision is pending', async () => {
    const decisionPromise = new Promise<ScreenerDecisionResponse>(() => undefined);
    const client = renderScreener(
      new ScreenerPageTestClient({
        decisionHandler: () => decisionPromise,
      }),
    );

    fireEvent.click(await screen.findByRole('button', { name: 'Deny' }));

    await waitFor(() => expect(client.decideScreenerCalls).toHaveLength(1));
    expect(screen.getByRole('button', { name: 'Saving…' })).toBeDisabled();
    expect(screen.getByRole('button', { name: 'Deny' })).toBeDisabled();
  });

  it('shows a decision error after choosing a routing destination', async () => {
    const client = renderScreener(
      new ScreenerPageTestClient({
        decisionHandler: () =>
          Promise.reject(new HailApiError(422, {}, response(422))),
      }),
    );

    fireEvent.click(await screen.findByRole('button', { name: 'Approve' }));
    fireEvent.click(screen.getByRole('menuitem', { name: 'Paper Trail' }));

    expect(
      await screen.findByRole('alert', {
        name: '',
      }),
    ).toHaveTextContent('The server rejected this decision. Refresh and try again.');
    expect(client.decideScreenerCalls[0]).toMatchObject({
      classify_as: 'papertrail',
      apply_to_history: true,
    });
  });

  it('shows an undo toast for denied senders when the API returns an undo token', async () => {
    const client = renderScreener(
      new ScreenerPageTestClient({
        decisionHandler: (body) =>
          Promise.resolve({
            sender: body.sender,
            decision: 'deny',
            undo: {
              id: 'undo-deny-1',
              action: 'screener.deny',
              expires_at: '2026-05-23T12:05:00Z',
            },
          }),
      }),
    );

    fireEvent.click(await screen.findByRole('button', { name: 'Deny' }));

    expect(await screen.findByText('Denied newsletter@example.com.')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Undo' })).toBeInTheDocument();
    expect(client.decideScreenerCalls).toHaveLength(1);
  });
});
