import { RouterProvider } from '@tanstack/react-router';
import { cleanup, fireEvent, screen, waitFor } from '@testing-library/react';
import type { ReactNode } from 'react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type {
  GmailConnectResponse,
  ProviderAccount,
  ProviderAccountResponse,
  ProviderSyncStatus,
  ProviderSyncStatusListResponse,
  ProviderSyncTriggerResponse,
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

const sampleSyncStatus: ProviderSyncStatus = {
  id: 42,
  provider_kind: 'gmail',
  provider_account_id: 'reader@gmail.com',
  provider_email: 'reader@gmail.com',
  display_email: 'Reader <reader@gmail.com>',
  sync_status: 'failed',
  last_sync_attempted_at: '2026-05-26T17:00:00Z',
  last_sync_succeeded_at: '2026-05-26T16:30:00Z',
  next_sync_after: '2026-05-26T17:15:00Z',
  sync_backoff_secs: 900,
  last_error_class: 'gmail_rate_limit',
  last_error_message: 'Gmail asked hail to slow down',
  last_profile_history_id: '12345',
  profile_synced_at: '2026-05-26T16:00:00Z',
  last_sync_event: {
    event_type: 'history_import',
    result_status: 'failed',
    safe_error_class: 'gmail_rate_limit',
    safe_error_message: 'Gmail asked hail to slow down',
    created_at: '2026-05-26T17:00:00Z',
  },
  last_error_event: {
    event_type: 'history_import',
    result_status: 'failed',
    safe_error_class: 'gmail_rate_limit',
    safe_error_message: 'Gmail asked hail to slow down',
    created_at: '2026-05-26T17:00:00Z',
  },
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
  syncStatusCalls = 0;
  triggerSyncCalls: number[] = [];
  syncStatuses: ProviderSyncStatus[] = [];
  connectFailure: Error | null = null;
  disconnectFailure: Error | null = null;
  syncStatusFailure: Error | null = null;
  triggerSyncFailure: Error | null = null;

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

  override async listProviderSyncStatuses(): Promise<ProviderSyncStatusListResponse> {
    this.syncStatusCalls += 1;
    if (this.syncStatusFailure) {
      throw this.syncStatusFailure;
    }
    return { accounts: this.syncStatuses };
  }

  override async triggerProviderSync(id: number): Promise<ProviderSyncTriggerResponse> {
    this.triggerSyncCalls.push(id);
    if (this.triggerSyncFailure) {
      throw this.triggerSyncFailure;
    }
    const account = {
      ...(this.syncStatuses.find((status) => status.id === id) ?? sampleSyncStatus),
      id,
      sync_status: 'active',
      next_sync_after: null,
      sync_backoff_secs: null,
    };
    this.syncStatuses = this.syncStatuses.map((status) => status.id === id ? account : status);
    return { account };
  }
}

let currentTestBody: ReactNode = null;
let restoreProviderAccountsRoute: (() => void) | null = null;

function TestBody() {
  return currentTestBody;
}

function installTestRouteComponent() {
  if (restoreProviderAccountsRoute) {
    return;
  }

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
  search = '',
  confirm = vi.fn(() => true),
}: {
  client?: ProviderAccountsTestClient;
  account?: ProviderAccount | null;
  assign?: (url: string) => void;
  search?: string;
  confirm?: (message: string) => boolean;
} = {}) {
  const queryClient = createTestQueryClient();
  seedMe(queryClient, client.testUser);
  currentTestBody = (
    <AuthProvider>
      <ProviderAccountsPage
        client={client}
        initialAccount={account}
        location={{ assign, search }}
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
    expect(await screen.findByText('No Gmail account connected')).toBeInTheDocument();

    clickButton('Connect Gmail');

    await waitFor(() => {
      expect(client.connectCalls).toBe(1);
      expect(assign).toHaveBeenCalledWith('https://accounts.google.test/oauth?state=abc');
    });
  });

  it('shows OAuth callback notices and refreshes Gmail import status', async () => {
    const connectedClient = new ProviderAccountsTestClient();
    connectedClient.syncStatuses = [sampleSyncStatus];
    renderPage({ client: connectedClient, search: '?connected=gmail' });

    expect(await screen.findByRole('status')).toHaveTextContent('Gmail connected. Hail is refreshing import status now.');
    await waitFor(() => {
      expect(connectedClient.syncStatusCalls).toBeGreaterThanOrEqual(1);
    });
    expect(screen.getByText('Gmail import health')).toBeInTheDocument();

    cleanup();

    renderPage({ search: '?error=oauth_exchange_failed&state=secret-state&code=secret-code' });
    expect(await screen.findByRole('alert')).toHaveTextContent('Gmail connection failed while exchanging authorization with Google. Please try again.');
    expect(screen.queryByText(/secret-state|secret-code/)).not.toBeInTheDocument();
  });

  it('shows connected Gmail status and disconnects with confirmation', async () => {
    const { client, confirm } = renderPage({ account: sampleAccount });

    expect(await screen.findByText('Reader <reader@gmail.com>')).toBeInTheDocument();
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

  it('does not disconnect when confirmation is cancelled', async () => {
    const { client, confirm } = renderPage({
      account: sampleAccount,
      confirm: vi.fn(() => false),
    });

    await screen.findByRole('button', { name: 'Disconnect' });
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

    await screen.findByText('Reader <reader@gmail.com>');
    clickButton('Disconnect');
    expect(await screen.findByText('Disconnect Gmail failed with HTTP 500.')).toBeInTheDocument();
  });

  it('shows Gmail sync health and triggers a manual sync', async () => {
    const client = new ProviderAccountsTestClient();
    client.syncStatuses = [sampleSyncStatus];
    renderPage({ client });

    expect(await screen.findByText('Gmail import health')).toBeInTheDocument();
    expect(screen.getByText('Needs attention')).toBeInTheDocument();
    expect(screen.getByText('Last successful sync')).toBeInTheDocument();
    expect(screen.getByText('Next retry')).toBeInTheDocument();
    expect(screen.getByText('15 minutes')).toBeInTheDocument();
    expect(screen.getByText('gmail_rate_limit: Gmail asked hail to slow down')).toBeInTheDocument();

    clickButton('Sync now');

    await waitFor(() => {
      expect(client.triggerSyncCalls).toEqual([42]);
      expect(screen.getByText('Connected')).toBeInTheDocument();
    });
  });

  it('surfaces sync status and manual sync errors', async () => {
    const statusClient = new ProviderAccountsTestClient();
    statusClient.syncStatusFailure = new HailApiError(503, undefined, response(503));
    renderPage({ client: statusClient });

    expect(await screen.findByText('Load Gmail import status failed with HTTP 503.')).toBeInTheDocument();

    cleanup();

    const syncClient = new ProviderAccountsTestClient();
    syncClient.syncStatuses = [sampleSyncStatus];
    syncClient.triggerSyncFailure = new HailApiError(500, undefined, response(500));
    renderPage({ client: syncClient });

    await screen.findByText('Gmail import health');
    clickButton('Sync now');
    expect(await screen.findByText('Sync Gmail now failed with HTTP 500.')).toBeInTheDocument();
  });
});
