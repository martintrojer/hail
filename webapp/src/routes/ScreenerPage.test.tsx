import { RouterProvider } from '@tanstack/react-router';
import { cleanup, fireEvent, screen, waitFor } from '@testing-library/react';
import type { ReactNode } from 'react';
import { afterEach, describe, expect, it } from 'vitest';
import type {
  DeniedSendersResponse,
  ScreenerDecisionRequest,
  ScreenerDecisionResponse,
  ScreenerView,
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
import { ScreenerPage } from './ScreenerPage';

class ScreenerPageTestClient extends TestHailApiClient {
  readonly decideScreenerCalls: ScreenerDecisionRequest[] = [];
  readonly undoDenyCalls: string[] = [];
  undoDenyFailure: Error | null = null;
  deniedFailure: Error | null = null;
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
    super({
      user: {
        id: 1,
        email: 'screener@example.com',
        display_name: 'Screener',
        is_admin: false,
      },
    });
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

  override async getScreenerView(): Promise<ScreenerView> {
    return this.viewPromise;
  }

  override async getDeniedSenders(): Promise<DeniedSendersResponse> {
    if (this.deniedFailure) {
      throw this.deniedFailure;
    }
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
    if (this.undoDenyFailure) {
      throw this.undoDenyFailure;
    }
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
  const queryClient = createTestQueryClient();

  seedMe(queryClient, client.testUser);

  currentTestBody = (
    <AuthProvider>
      <UndoToastProvider>
        <ScreenerPage client={client} />
      </UndoToastProvider>
    </AuthProvider>
  );
  installTestRouteComponent();
  window.history.pushState({}, '', '/screener');

  renderWithQueryClient(<RouterProvider router={router} />, queryClient);

  return client;
}

function sampleScreenerView(
  overrides: Partial<ScreenerView> = {},
): ScreenerView {
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
        emails: [
          {
            email_id: 'email-3',
            subject: 'Newsletter dispatch',
            preview: 'Latest dispatch from the newsletter.',
            received_at: '2026-05-23T12:15:00Z',
          },
          {
            email_id: 'email-2',
            subject: 'Earlier dispatch',
            preview: 'Earlier newsletter issue.',
            received_at: '2026-05-22T12:15:00Z',
          },
          {
            email_id: 'email-1',
            subject: '',
            preview: '',
            received_at: null,
          },
        ],
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
  it('expands a sender card to show all pending emails', async () => {
    renderScreener();

    expect(await screen.findByText('Newsletter dispatch')).toBeInTheDocument();
    expect(screen.queryByText('Earlier dispatch')).not.toBeInTheDocument();

    fireEvent.click(
      screen.getByRole('button', { name: /Show · 3 pending emails/i }),
    );

    expect(
      screen.getByRole('button', { name: /Hide · 3 pending emails/i }),
    ).toHaveAttribute('aria-expanded', 'true');
    expect(screen.getByText('Earlier dispatch')).toBeInTheDocument();
    expect(screen.getByText('Earlier newsletter issue.')).toBeInTheDocument();
    expect(screen.getByText('May 22, 2026')).toBeInTheDocument();
    expect(screen.getByText('No subject')).toBeInTheDocument();
    expect(screen.getByText('Preview unavailable.')).toBeInTheDocument();
    expect(screen.getByText('Date unavailable')).toBeInTheDocument();
  });

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

    expect(
      screen.getByRole('button', { name: /Previously denied/i }),
    ).toHaveAttribute('aria-expanded', 'false');
    expect(screen.queryByText('blocked@example.com')).not.toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: /Previously denied/i }));

    expect(await screen.findByText('blocked@example.com')).toBeInTheDocument();
    expect(screen.getByText(/Denied/)).toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: 'Undo' }));

    await waitFor(() =>
      expect(client.undoDenyCalls).toEqual(['blocked@example.com']),
    );
    expect(
      await screen.findByText('Restored blocked@example.com to the Screener.'),
    ).toBeInTheDocument();
  });

  it('loads denied senders only after expanding the section', async () => {
    const client = renderScreener(
      new ScreenerPageTestClient({ denied: sampleDeniedSenders({ denied: [] }) }),
    );

    await screen.findByText('Newsletter dispatch');
    expect(client.undoDenyCalls).toEqual([]);
    expect(screen.queryByLabelText('Loading denied senders')).not.toBeInTheDocument();
    expect(screen.queryByText('No denied senders yet.')).not.toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: /Previously denied/i }));

    expect(await screen.findByText('No denied senders yet.')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: /Previously denied/i })).toHaveTextContent(
      'Hide',
    );
  });

  it('shows denied sender loading and error states inside the expanded section', async () => {
    renderScreener(
      new ScreenerPageTestClient({
        deniedPromise: new Promise<DeniedSendersResponse>(() => undefined),
      }),
    );

    fireEvent.click(await screen.findByRole('button', { name: /Previously denied/i }));

    expect(screen.getByLabelText('Loading denied senders')).toBeInTheDocument();
    cleanup();
    restoreRoute();

    const errorClient = new ScreenerPageTestClient();
    errorClient.deniedFailure = new HailApiError(503, undefined, response(503));
    renderScreener(errorClient);

    fireEvent.click(await screen.findByRole('button', { name: /Previously denied/i }));

    expect(await screen.findByText('Something went wrong.')).toBeInTheDocument();
    expect(
      screen.getByText('Screener failed with HTTP 503.'),
    ).toBeInTheDocument();
  });

  it('shows an inline error and keeps the denied row when undo fails', async () => {
    const client = new ScreenerPageTestClient();
    client.undoDenyFailure = new HailApiError(422, undefined, response(422));
    renderScreener(client);

    fireEvent.click(await screen.findByRole('button', { name: /Previously denied/i }));
    expect(await screen.findByText('blocked@example.com')).toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: 'Undo' }));

    await waitFor(() =>
      expect(client.undoDenyCalls).toEqual(['blocked@example.com']),
    );
    expect(
      await screen.findByText('The server rejected this decision. Refresh and try again.'),
    ).toBeInTheDocument();
    expect(screen.getByText('blocked@example.com')).toBeInTheDocument();
  });

  it('shows initial pending, error, and empty states', async () => {
    const neverResolves = new Promise<ScreenerView>(() => undefined);
    const pendingClient = renderScreener(
      new ScreenerPageTestClient({ viewPromise: neverResolves }),
    );

    expect(
      screen.getByLabelText('Loading pending senders'),
    ).toBeInTheDocument();
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
      screen.getByText(
        'Your session expired. Sign in again to refresh the Screener.',
      ),
    ).toBeInTheDocument();
    cleanup();
    restoreRoute();

    renderScreener(
      new ScreenerPageTestClient({ view: sampleScreenerView({ senders: [] }) }),
    );
    expect(await screen.findByText('No unknown senders')).toBeInTheDocument();
  });

  it('disables card controls while a decision is pending', async () => {
    const decisionPromise = new Promise<ScreenerDecisionResponse>(
      () => undefined,
    );
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
    ).toHaveTextContent(
      'The server rejected this decision. Refresh and try again.',
    );
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

    expect(
      await screen.findByText('Denied newsletter@example.com.'),
    ).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Undo' })).toBeInTheDocument();
    expect(client.decideScreenerCalls).toHaveLength(1);
  });
});
