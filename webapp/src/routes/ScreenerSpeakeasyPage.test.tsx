import { RouterProvider } from '@tanstack/react-router';
import { cleanup, fireEvent, screen, waitFor } from '@testing-library/react';
import type { ReactNode } from 'react';
import { afterEach, describe, expect, it } from 'vitest';
import type {
  ScreenerAllowedView,
  ScreenerClassification,
  ScreenerDecisionRequest,
  ScreenerDecisionResponse,
} from '../api/client';
import { HailApiError } from '../api/client';
import { AuthProvider } from '../auth/AuthProvider';
import { router } from '../router';
import {
  createTestQueryClient,
  renderWithQueryClient,
  seedMe,
  TestHailApiClient,
} from '../test-utils';
import { ScreenerSpeakeasyPage } from './ScreenerSpeakeasyPage';

class ScreenerSpeakeasyPageTestClient extends TestHailApiClient {
  readonly decideScreenerCalls: ScreenerDecisionRequest[] = [];
  allowedFailure: Error | null = null;
  decisionFailure: Error | null = null;
  private allowedPromise: Promise<ScreenerAllowedView>;

  constructor({
    allowed = sampleAllowedView(),
    allowedPromise,
  }: {
    allowed?: ScreenerAllowedView;
    allowedPromise?: Promise<ScreenerAllowedView>;
  } = {}) {
    super();
    this.allowedPromise = allowedPromise ?? Promise.resolve(allowed);
  }

  override async getScreenerAllowedView(): Promise<ScreenerAllowedView> {
    if (this.allowedFailure) {
      throw this.allowedFailure;
    }
    return this.allowedPromise;
  }

  override async decideScreener(
    body: ScreenerDecisionRequest,
  ): Promise<ScreenerDecisionResponse> {
    this.decideScreenerCalls.push(body);
    if (this.decisionFailure) {
      throw this.decisionFailure;
    }
    return {
      sender: body.sender,
      decision: body.decision,
      classify_as: body.classify_as as ScreenerClassification,
    };
  }
}

let currentTestBody: ReactNode = null;
let restoreSpeakeasyRoute: (() => void) | null = null;

function restoreRoute() {
  restoreSpeakeasyRoute?.();
  restoreSpeakeasyRoute = null;
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
  const matchRoute = router.routesByPath['/screener/speakeasy'];
  const previousComponent = matchRoute.options.component;
  const previousBeforeLoad = matchRoute.options.beforeLoad;
  matchRoute.options.component = TestBody;
  matchRoute.options.beforeLoad = undefined;
  restoreSpeakeasyRoute = () => {
    matchRoute.options.component = previousComponent;
    matchRoute.options.beforeLoad = previousBeforeLoad;
  };
}

function renderSpeakeasy(client = new ScreenerSpeakeasyPageTestClient()) {
  const queryClient = createTestQueryClient();

  seedMe(queryClient, client.testUser);

  currentTestBody = (
    <AuthProvider>
      <ScreenerSpeakeasyPage client={client} />
    </AuthProvider>
  );
  installTestRouteComponent();
  window.history.pushState({}, '', '/screener/speakeasy');

  renderWithQueryClient(<RouterProvider router={router} />, queryClient);

  return client;
}

function sampleAllowedView(
  overrides: Partial<ScreenerAllowedView> = {},
): ScreenerAllowedView {
  return {
    allowed: [
      {
        sender_address: 'friend@example.com',
        classify_as: 'imbox',
        first_seen_at: '2026-05-20T09:00:00Z',
        decided_at: '2026-05-21T10:00:00Z',
      },
      {
        sender_address: 'receipts@example.com',
        classify_as: 'papertrail',
        first_seen_at: '2026-05-22T11:00:00Z',
        decided_at: null,
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

describe('ScreenerSpeakeasyPage', () => {
  it('renders approved senders with their API-provided classifications', async () => {
    renderSpeakeasy();

    expect(await screen.findByRole('heading', { name: 'Speakeasy' })).toBeInTheDocument();
    expect(await screen.findByText('friend@example.com')).toBeInTheDocument();
    expect(screen.getByText('Routed to The Imbox')).toBeInTheDocument();
    expect(screen.getByText('First seen May 20, 2026')).toBeInTheDocument();
    expect(screen.getByText('Approved May 21, 2026')).toBeInTheDocument();
    expect(screen.getByText('receipts@example.com')).toBeInTheDocument();
    expect(screen.getByText('Routed to Paper Trail')).toBeInTheDocument();
    expect(screen.getByText('2 approved senders')).toBeInTheDocument();
  });

  it('changes an approved sender route through the shared routing picker', async () => {
    const client = renderSpeakeasy();

    fireEvent.click(await screen.findAllByRole('button', { name: 'Change route' }).then((buttons) => buttons[0]));
    expect(
      screen.getByRole('menu', { name: 'Screener routing destinations' }),
    ).toBeInTheDocument();

    fireEvent.click(screen.getByRole('menuitem', { name: 'The Feed' }));

    await waitFor(() => expect(client.decideScreenerCalls).toHaveLength(1));
    expect(client.decideScreenerCalls[0]).toEqual({
      sender: 'friend@example.com',
      decision: 'approve',
      classify_as: 'feed',
      apply_to_history: true,
    });
  });

  it('shows loading, empty, and error states', async () => {
    renderSpeakeasy(
      new ScreenerSpeakeasyPageTestClient({
        allowedPromise: new Promise<ScreenerAllowedView>(() => undefined),
      }),
    );
    expect(screen.getByLabelText('Loading approved senders')).toBeInTheDocument();
    cleanup();
    restoreRoute();

    renderSpeakeasy(
      new ScreenerSpeakeasyPageTestClient({
        allowed: sampleAllowedView({ allowed: [] }),
      }),
    );
    expect(await screen.findByText('No approved senders yet.')).toBeInTheDocument();
    cleanup();
    restoreRoute();

    const errorClient = new ScreenerSpeakeasyPageTestClient();
    errorClient.allowedFailure = new HailApiError(503, undefined, response(503));
    renderSpeakeasy(errorClient);
    expect(await screen.findByText('Something went wrong.')).toBeInTheDocument();
    expect(
      screen.getByText('Screener Speakeasy failed with HTTP 503.'),
    ).toBeInTheDocument();
  });

  it('shows an inline error when changing a route fails', async () => {
    const client = new ScreenerSpeakeasyPageTestClient();
    client.decisionFailure = new HailApiError(422, undefined, response(422));
    renderSpeakeasy(client);

    fireEvent.click(await screen.findAllByRole('button', { name: 'Change route' }).then((buttons) => buttons[0]));
    fireEvent.click(screen.getByRole('menuitem', { name: 'Paper Trail' }));

    await waitFor(() => expect(client.decideScreenerCalls).toHaveLength(1));
    expect(
      await screen.findByText('The server rejected this decision. Refresh and try again.'),
    ).toBeInTheDocument();
  });
});
