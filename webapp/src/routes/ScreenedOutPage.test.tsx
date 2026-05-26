import { RouterProvider } from '@tanstack/react-router';
import { cleanup, fireEvent, screen, waitFor } from '@testing-library/react';
import type { ReactNode } from 'react';
import { afterEach, describe, expect, it } from 'vitest';
import type {
  DeniedSendersResponse,
  ScreenerClassification,
  UndoDenyRequest,
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
import { ScreenedOutPage } from './ScreenedOutPage';

class ScreenedOutPageTestClient extends TestHailApiClient {
  readonly undoDenyCalls: Array<{ address: string; body?: UndoDenyRequest }> = [];
  deniedFailure: Error | null = null;
  undoFailure: Error | null = null;
  private deniedPromise: Promise<DeniedSendersResponse>;

  constructor({
    denied = sampleDeniedSenders(),
    deniedPromise,
  }: {
    denied?: DeniedSendersResponse;
    deniedPromise?: Promise<DeniedSendersResponse>;
  } = {}) {
    super();
    this.deniedPromise = deniedPromise ?? Promise.resolve(denied);
  }

  override async getDeniedSenders(): Promise<DeniedSendersResponse> {
    if (this.deniedFailure) {
      throw this.deniedFailure;
    }
    return this.deniedPromise;
  }

  override async undoDeny(address: string, body?: UndoDenyRequest) {
    this.undoDenyCalls.push({ address, body });
    if (this.undoFailure) {
      throw this.undoFailure;
    }
    return {
      status: 'approved' as const,
      classify_as: (body?.classify_as ?? 'imbox') as ScreenerClassification,
    };
  }
}

let currentTestBody: ReactNode = null;
let restoreScreenedOutRoute: (() => void) | null = null;

function restoreRoute() {
  restoreScreenedOutRoute?.();
  restoreScreenedOutRoute = null;
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
  const matchRoute = router.routesByPath['/screened-out'];
  const previousComponent = matchRoute.options.component;
  const previousBeforeLoad = matchRoute.options.beforeLoad;
  matchRoute.options.component = TestBody;
  matchRoute.options.beforeLoad = undefined;
  restoreScreenedOutRoute = () => {
    matchRoute.options.component = previousComponent;
    matchRoute.options.beforeLoad = previousBeforeLoad;
  };
}

function renderScreenedOut(client = new ScreenedOutPageTestClient()) {
  const queryClient = createTestQueryClient();

  seedMe(queryClient, client.testUser);

  currentTestBody = (
    <AuthProvider>
      <ScreenedOutPage client={client} />
    </AuthProvider>
  );
  installTestRouteComponent();
  window.history.pushState({}, '', '/screened-out');

  renderWithQueryClient(<RouterProvider router={router} />, queryClient);

  return client;
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

describe('ScreenedOutPage', () => {
  it('renders screened-out and blocked sender sections from denied senders', async () => {
    renderScreenedOut();

    expect(await screen.findByText('Screened-out emails')).toBeInTheDocument();
    expect(screen.getByText('Blocked senders')).toBeInTheDocument();
    expect(screen.getAllByText('blocked@example.com')).toHaveLength(2);
    expect(screen.getAllByText('Denied May 22, 2026')).toHaveLength(2);
  });

  it('allows a sender through the routing picker and refreshes the list', async () => {
    const client = renderScreenedOut();

    fireEvent.click(await screen.findAllByRole('button', { name: 'Allow' }).then((buttons) => buttons[0]));
    expect(
      screen.getByRole('menu', { name: 'Screener routing destinations' }),
    ).toBeInTheDocument();

    fireEvent.click(screen.getByRole('menuitem', { name: 'Paper Trail' }));

    await waitFor(() => expect(client.undoDenyCalls).toHaveLength(1));
    expect(client.undoDenyCalls[0]).toEqual({
      address: 'blocked@example.com',
      body: { classify_as: 'papertrail' },
    });
  });

  it('shows loading, empty, and error states', async () => {
    renderScreenedOut(
      new ScreenedOutPageTestClient({
        deniedPromise: new Promise<DeniedSendersResponse>(() => undefined),
      }),
    );
    expect(screen.getByLabelText('Loading screened-out senders')).toBeInTheDocument();
    cleanup();
    restoreRoute();

    renderScreenedOut(
      new ScreenedOutPageTestClient({ denied: sampleDeniedSenders({ denied: [] }) }),
    );
    expect(await screen.findByText('No screened-out emails.')).toBeInTheDocument();
    expect(screen.getByText('No blocked senders.')).toBeInTheDocument();
    cleanup();
    restoreRoute();

    const errorClient = new ScreenedOutPageTestClient();
    errorClient.deniedFailure = new HailApiError(503, undefined, response(503));
    renderScreenedOut(errorClient);
    expect(await screen.findByText('Something went wrong.')).toBeInTheDocument();
    expect(
      screen.getByText('Screened Out failed with HTTP 503.'),
    ).toBeInTheDocument();
  });

  it('shows an inline error when allowing a sender fails', async () => {
    const client = new ScreenedOutPageTestClient();
    client.undoFailure = new HailApiError(422, undefined, response(422));
    renderScreenedOut(client);

    fireEvent.click(await screen.findAllByRole('button', { name: 'Allow' }).then((buttons) => buttons[0]));
    fireEvent.click(screen.getByRole('menuitem', { name: 'The Feed' }));

    await waitFor(() => expect(client.undoDenyCalls).toHaveLength(1));
    expect(
      await screen.findByText('The server rejected this decision. Refresh and try again.'),
    ).toBeInTheDocument();
  });
});
