import { RouterProvider } from '@tanstack/react-router';
import { cleanup, fireEvent, screen, waitFor } from '@testing-library/react';
import type { ReactNode } from 'react';
import { afterEach, describe, expect, it } from 'vitest';
import type {
  RotateSpeakeasyRequest,
  RotateSpeakeasyResponse,
  SpeakeasyResponse,
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
  readonly rotateCalls: RotateSpeakeasyRequest[] = [];
  speakeasyFailure: Error | null = null;
  rotateFailure: Error | null = null;
  private speakeasyPromise: Promise<SpeakeasyResponse>;
  private rotated: SpeakeasyResponse;

  constructor({
    speakeasy = sampleSpeakeasyResponse(),
    speakeasyPromise,
    rotated = sampleSpeakeasyResponse({
      passphrase: 'new-river-copper-saffron-willow-0123456789abcdef',
      generated_at: '2026-05-27T12:00:00Z',
      manually_rotated_at: '2026-05-27T12:00:00Z',
    }),
  }: {
    speakeasy?: SpeakeasyResponse;
    speakeasyPromise?: Promise<SpeakeasyResponse>;
    rotated?: SpeakeasyResponse;
  } = {}) {
    super();
    this.speakeasyPromise = speakeasyPromise ?? Promise.resolve(speakeasy);
    this.rotated = rotated;
  }

  override async getSpeakeasy(): Promise<SpeakeasyResponse> {
    if (this.speakeasyFailure) {
      throw this.speakeasyFailure;
    }
    return this.speakeasyPromise;
  }

  override async rotateSpeakeasy(
    body: RotateSpeakeasyRequest = { acknowledge_bypass_secret: true },
  ): Promise<RotateSpeakeasyResponse> {
    this.rotateCalls.push(body);
    if (this.rotateFailure) {
      throw this.rotateFailure;
    }
    this.speakeasyPromise = Promise.resolve(this.rotated);
    return this.rotated;
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

function sampleSpeakeasyResponse(
  overrides: Partial<SpeakeasyResponse['speakeasy']> = {},
): SpeakeasyResponse {
  return {
    speakeasy: {
      passphrase: 'amber-basil-canyon-delta-abcdef0123456789',
      period: '2026-05',
      rotates_at: '2026-06-01T00:00:00Z',
      generated_at: '2026-05-01T00:00:00Z',
      manually_rotated_at: null,
      ...overrides,
    },
  };
}

function response(status: number, body: unknown = {}) {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

describe('ScreenerSpeakeasyPage', () => {
  it('renders the current monthly passphrase with one-message bypass semantics', async () => {
    renderSpeakeasy();

    expect(await screen.findByRole('heading', { name: 'Speakeasy' })).toBeInTheDocument();
    expect(
      await screen.findByLabelText('Current Speakeasy passphrase'),
    ).toHaveValue('amber-basil-canyon-delta-abcdef0123456789');
    expect(
      screen.getByRole('heading', {
        name: 'A monthly passphrase for one-message bypasses.',
      }),
    ).toBeInTheDocument();
    expect(screen.getByText('May 2026')).toBeInTheDocument();
    expect(screen.getByText('One message only')).toBeInTheDocument();
    expect(
      screen.getByText(/A matching message skips the Screener once/),
    ).toBeInTheDocument();
    expect(
      screen.getByText(/does not approve the sender, create a rule/),
    ).toBeInTheDocument();
    expect(screen.queryByText(/approved senders/i)).not.toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Change route' })).not.toBeInTheDocument();
  });

  it('regenerates the passphrase through the Speakeasy API', async () => {
    const client = renderSpeakeasy();

    fireEvent.click(
      await screen.findByRole('button', { name: 'Regenerate passphrase' }),
    );

    await waitFor(() => expect(client.rotateCalls).toHaveLength(1));
    expect(client.rotateCalls[0]).toEqual({
      acknowledge_bypass_secret: true,
    });
    expect(
      await screen.findByDisplayValue(
        'new-river-copper-saffron-willow-0123456789abcdef',
      ),
    ).toBeInTheDocument();
    expect(screen.getByText(/The old phrase stops working immediately/)).toBeInTheDocument();
  });

  it('shows loading and error states for the passphrase API', async () => {
    renderSpeakeasy(
      new ScreenerSpeakeasyPageTestClient({
        speakeasyPromise: new Promise<SpeakeasyResponse>(() => undefined),
      }),
    );
    expect(screen.getByLabelText('Loading Speakeasy passphrase')).toBeInTheDocument();
    cleanup();
    restoreRoute();

    const errorClient = new ScreenerSpeakeasyPageTestClient();
    errorClient.speakeasyFailure = new HailApiError(503, undefined, response(503));
    renderSpeakeasy(errorClient);
    expect(await screen.findByText('Something went wrong.')).toBeInTheDocument();
    expect(screen.getByText('Speakeasy failed with HTTP 503.')).toBeInTheDocument();
  });

  it('shows an inline error when regeneration fails', async () => {
    const client = new ScreenerSpeakeasyPageTestClient();
    client.rotateFailure = new HailApiError(403, undefined, response(403));
    renderSpeakeasy(client);

    fireEvent.click(
      await screen.findByRole('button', { name: 'Regenerate passphrase' }),
    );

    await waitFor(() => expect(client.rotateCalls).toHaveLength(1));
    expect(
      await screen.findByText('Speakeasy rotation failed with HTTP 403.'),
    ).toBeInTheDocument();
  });
});
