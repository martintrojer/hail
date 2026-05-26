import { RouterProvider } from '@tanstack/react-router';
import { cleanup, fireEvent, screen, waitFor } from '@testing-library/react';
import type { ReactNode } from 'react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type {
  GmailConnectResponse,
  ProviderAccount,
  ProviderAccountResponse,
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
import { ProviderAccountsPage } from './ProviderAccountsPage';

const sampleAccount: ProviderAccount = {
  id: 42,
  provider_kind: 'gmail',
  provider_account_id: 'reader@gmail.com',
  provider_email: 'reader@gmail.com',
  display_email: 'Reader <reader@gmail.com>',
  granted_scopes: ['https://www.googleapis.com/auth/gmail.readonly'],
  sync_status: 'active',
  cached_access_token_expires_at: '2026-05-26T18:00:00Z',
  last_profile_history_id: '12345',
};

function response(status: number) {
  return new Response(JSON.stringify({ error: 'boom' }), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

class ProviderAccountsTestClient extends TestHailApiClient {
  connectCalls = 0;
  disconnectCalls: number[] = [];
  connectFailure: Error | null = null;
  disconnectFailure: Error | null = null;

  override async connectGmail(): Promise<GmailConnectResponse> {
    this.connectCalls += 1;
    if (this.connectFailure) {
      throw this.connectFailure;
    }
    return {
      authorization_url: 'https://accounts.google.test/oauth?state=abc',
      scopes: ['https://www.googleapis.com/auth/gmail.readonly'],
    };
  }

  override async disconnectProviderAccount(id: number): Promise<ProviderAccountResponse> {
    this.disconnectCalls.push(id);
    if (this.disconnectFailure) {
      throw this.disconnectFailure;
    }
    return {
      ...sampleAccount,
      id,
      sync_status: 'disconnected',
    };
  }
}

let currentTestBody: ReactNode = null;
let restoreProviderAccountsRoute: (() => void) | null = null;

function TestBody() {
  return currentTestBody;
}

function installTestRouteComponent() {
  const matchRoute = router.routesByPath['/provider-accounts'];
  const previousComponent = matchRoute.options.component;
  const previousBeforeLoad = matchRoute.options.beforeLoad;
  matchRoute.options.component = TestBody;
  matchRoute.options.beforeLoad = undefined;
  restoreProviderAccountsRoute = () => {
    matchRoute.options.component = previousComponent;
    matchRoute.options.beforeLoad = previousBeforeLoad;
  };
}

function restoreRoute() {
  restoreProviderAccountsRoute?.();
  restoreProviderAccountsRoute = null;
}

function renderPage({
  client = new ProviderAccountsTestClient(),
  account = null,
  assign = vi.fn(),
  confirm = vi.fn(() => true),
}: {
  client?: ProviderAccountsTestClient;
  account?: ProviderAccount | null;
  assign?: (url: string) => void;
  confirm?: (message: string) => boolean;
} = {}) {
  const queryClient = createTestQueryClient();
  seedMe(queryClient, client.testUser);
  currentTestBody = (
    <AuthProvider>
      <ProviderAccountsPage
        client={client}
        initialAccount={account}
        location={{ assign }}
        confirmDisconnect={confirm}
      />
    </AuthProvider>
  );
  installTestRouteComponent();
  window.history.pushState({}, '', '/provider-accounts');
  renderWithQueryClient(<RouterProvider router={router} />, queryClient);
  return { client, assign, confirm };
}

function clickButton(name: string | RegExp) {
  fireEvent.click(screen.getByRole('button', { name }));
}

afterEach(() => {
  currentTestBody = null;
  restoreRoute();
  window.history.pushState({}, '', '/');
  cleanup();
  vi.restoreAllMocks();
});

describe('ProviderAccountsPage', () => {
  it('opens the Gmail authorization URL returned by the API client', async () => {
    const { client, assign } = renderPage();

    expect(await screen.findByRole('heading', { name: 'Provider Accounts' })).toBeInTheDocument();
    expect(screen.getByText('No Gmail account connected')).toBeInTheDocument();

    clickButton('Connect Gmail');

    await waitFor(() => {
      expect(client.connectCalls).toBe(1);
      expect(assign).toHaveBeenCalledWith('https://accounts.google.test/oauth?state=abc');
    });
  });

  it('shows connected Gmail status and disconnects with confirmation', async () => {
    const { client, confirm } = renderPage({ account: sampleAccount });

    expect(screen.getByText('Reader <reader@gmail.com>')).toBeInTheDocument();
    expect(screen.getByText('Connected')).toBeInTheDocument();
    expect(screen.getByText('Gmail read-only import')).toBeInTheDocument();
    expect(screen.getByText('12345')).toBeInTheDocument();

    clickButton('Disconnect');

    await waitFor(() => {
      expect(confirm).toHaveBeenCalledWith(expect.stringContaining('reader@gmail.com'));
      expect(client.disconnectCalls).toEqual([42]);
      expect(screen.getAllByText('Disconnected').length).toBeGreaterThan(0);
    });
  });

  it('does not disconnect when confirmation is cancelled', () => {
    const { client, confirm } = renderPage({
      account: sampleAccount,
      confirm: vi.fn(() => false),
    });

    clickButton('Disconnect');

    expect(confirm).toHaveBeenCalled();
    expect(client.disconnectCalls).toEqual([]);
  });

  it('surfaces client errors for connect and disconnect actions', async () => {
    const connectClient = new ProviderAccountsTestClient();
    connectClient.connectFailure = new HailApiError(503, undefined, response(503));
    renderPage({ client: connectClient });

    clickButton('Connect Gmail');
    expect(await screen.findByText('Connect Gmail failed with HTTP 503.')).toBeInTheDocument();

    cleanup();

    const disconnectClient = new ProviderAccountsTestClient();
    disconnectClient.disconnectFailure = new HailApiError(500, undefined, response(500));
    renderPage({ client: disconnectClient, account: sampleAccount });

    clickButton('Disconnect');
    expect(await screen.findByText('Disconnect Gmail failed with HTTP 500.')).toBeInTheDocument();
  });
});
