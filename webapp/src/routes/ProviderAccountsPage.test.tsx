import { QueryClient } from '@tanstack/react-query';
import { RouterProvider } from '@tanstack/react-router';
import {
  cleanup,
  fireEvent,
  screen,
  waitFor,
  within,
} from '@testing-library/react';
import type { ReactNode } from 'react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type {
  GmailConnectResponse,
  ProviderAccount,
  ProviderAccountResponse,
  ProviderReimportResponse,
  ProviderStopSyncResponse,
  ProviderSyncStatus,
  ProviderSyncStatusListResponse,
  ProviderSyncTriggerResponse,
} from '../api/client';
import { HailApiClient, HailApiError } from '../api/client';
import { queryKeys } from '../api/queryKeys';
import { AuthProvider } from '../auth/AuthProvider';
import { router } from '../router';
import {
  createTestQueryClient,
  renderWithQueryClient,
  seedMe,
  TestHailApiClient,
} from '../test-utils';
import { ProviderAccountsPage } from './ProviderAccountsPage';

function providerAccount(
  overrides: Partial<ProviderAccount> = {},
): ProviderAccount {
  return {
    id: 42,
    provider_kind: 'gmail',
    provider_account_id: 'reader@gmail.com',
    provider_email: 'reader@gmail.com',
    display_email: 'Reader <reader@gmail.com>',
    granted_scopes: ['https://www.googleapis.com/auth/gmail.readonly', 'https://www.googleapis.com/auth/gmail.send'],
    sync_status: 'active',
    cached_access_token_expires_at: '2026-05-26T18:00:00Z',
    last_profile_history_id: '12345',
    ...overrides,
  };
}

function providerSyncStatus(
  overrides: Partial<ProviderSyncStatus> = {},
): ProviderSyncStatus {
  return {
    id: 42,
    provider_kind: 'gmail',
    provider_account_id: 'reader@gmail.com',
    provider_email: 'reader@gmail.com',
    display_email: 'Reader <reader@gmail.com>',
    sync_status: 'error',
    last_sync_attempted_at: '2026-05-26T17:00:00Z',
    last_sync_succeeded_at: '2026-05-26T17:00:00Z',
    next_sync_after: '2026-05-26T17:15:00Z',
    sync_backoff_secs: 900,
    last_error_class: 'gmail_rate_limit',
    last_error_message: null,
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
    ...overrides,
  };
}

const sampleSyncStatus = providerSyncStatus();

function response(status: number) {
  return new Response(JSON.stringify({ error: 'boom' }), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

function jsonResponse(status: number, body: unknown) {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

function deferred<T>() {
  let resolve!: (value: T | PromiseLike<T>) => void;
  const promise = new Promise<T>((promiseResolve) => {
    resolve = promiseResolve;
  });
  return { promise, resolve };
}

class ProviderAccountsTestClient extends TestHailApiClient {
  connectCalls = 0;
  disconnectCalls: number[] = [];
  syncStatusCalls = 0;
  triggerSyncCalls: number[] = [];
  reimportCalls: number[] = [];
  stopCalls: number[] = [];
  syncStatuses: ProviderSyncStatus[] = [];
  syncStatusResponses: ProviderSyncStatusListResponse[] = [];
  connectResponse: GmailConnectResponse = {
    authorization_url: 'https://accounts.google.test/oauth?state=abc',
    scopes: ['https://www.googleapis.com/auth/gmail.readonly'],
  };
  disconnectResponse: ProviderAccountResponse | null = null;
  triggerSyncResponse: ProviderSyncTriggerResponse | null = null;
  reimportResponse: ProviderReimportResponse | null = null;
  triggerSyncPromise: Promise<ProviderSyncTriggerResponse> | null = null;
  stopResponse: ProviderStopSyncResponse | null = null;
  connectFailure: Error | null = null;
  disconnectFailure: Error | null = null;
  syncStatusFailure: Error | null = null;
  triggerSyncFailure: Error | null = null;
  reimportFailure: Error | null = null;
  stopFailure: Error | null = null;

  override async connectGmail(): Promise<GmailConnectResponse> {
    this.connectCalls += 1;
    if (this.connectFailure) {
      throw this.connectFailure;
    }
    return this.connectResponse;
  }

  override async disconnectProviderAccount(
    id: number,
  ): Promise<ProviderAccountResponse> {
    this.disconnectCalls.push(id);
    if (this.disconnectFailure) {
      throw this.disconnectFailure;
    }
    const updated = (
      this.disconnectResponse ??
      providerAccount({ id, sync_status: 'disconnected' })
    );
    this.syncStatuses = this.syncStatuses.map((status) =>
      status.id === id
        ? {
            ...status,
            display_email: updated.display_email,
            last_profile_history_id: updated.last_profile_history_id,
            provider_account_id: updated.provider_account_id,
            provider_email: updated.provider_email,
            provider_kind: updated.provider_kind,
            sync_status: updated.sync_status,
          }
        : status,
    );
    return updated;
  }

  override async listProviderSyncStatuses(): Promise<ProviderSyncStatusListResponse> {
    this.syncStatusCalls += 1;
    if (this.syncStatusFailure) {
      throw this.syncStatusFailure;
    }
    const response = this.syncStatusResponses.shift() ?? {
      accounts: this.syncStatuses,
    };
    this.syncStatuses = response.accounts;
    return response;
  }

  override async triggerProviderSync(
    id: number,
  ): Promise<ProviderSyncTriggerResponse> {
    this.triggerSyncCalls.push(id);
    if (this.triggerSyncFailure) {
      throw this.triggerSyncFailure;
    }
    if (this.triggerSyncPromise) {
      return this.triggerSyncPromise;
    }
    const account = this.triggerSyncResponse?.account ?? providerSyncStatus({
      ...(this.syncStatuses.find((status) => status.id === id) ?? {}),
      id,
      sync_status: 'active',
      next_sync_after: null,
      sync_backoff_secs: null,
    });
    this.syncStatuses = this.syncStatuses.map((status) => status.id === id ? account : status);
    return { account };
  }

  override async reimportProviderAccount(
    id: number,
  ): Promise<ProviderReimportResponse> {
    this.reimportCalls.push(id);
    if (this.reimportFailure) {
      throw this.reimportFailure;
    }
    const account = this.reimportResponse?.account ?? providerSyncStatus({
      ...(this.syncStatuses.find((status) => status.id === id) ?? {}),
      id,
      sync_status: 'initial_sync',
      next_sync_after: null,
      sync_backoff_secs: null,
      last_error_class: null,
      last_error_message: null,
      last_error_event: null,
      last_profile_history_id: null,
    });
    this.syncStatuses = this.syncStatuses.map((status) => status.id === id ? account : status);
    return { account };
  }

  override async stopProviderSync(
    id: number,
  ): Promise<ProviderStopSyncResponse> {
    this.stopCalls.push(id);
    if (this.stopFailure) {
      throw this.stopFailure;
    }
    const account = this.stopResponse?.account ?? providerSyncStatus({
      ...(this.syncStatuses.find((status) => status.id === id) ?? {}),
      id,
      sync_status: 'paused',
      next_sync_after: null,
      sync_backoff_secs: null,
      last_error_class: 'operator_paused',
      last_error_message: null,
      last_error_event: {
        event_type: 'sync_paused',
        result_status: 'info',
        safe_error_class: 'operator_paused',
        safe_error_message: 'Gmail import paused by operator',
        created_at: '2026-05-26T18:01:00Z',
      },
    });
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
  queryClient = createTestQueryClient(),
}: {
  client?: HailApiClient;
  account?: ProviderSyncStatus | null;
  assign?: (url: string) => void;
  search?: string;
  confirm?: (message: string) => boolean;
  queryClient?: QueryClient;
} = {}) {
  seedMe(queryClient);
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
  return { client, assign, confirm, queryClient };
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

    expect(
      await screen.findByRole('heading', { name: 'Provider Accounts' }),
    ).toBeInTheDocument();
    expect(
      await screen.findByText('No Gmail account connected'),
    ).toBeInTheDocument();

    clickButton('Connect Gmail');

    await waitFor(() => {
      expect((client as ProviderAccountsTestClient).connectCalls).toBe(1);
      expect(assign).toHaveBeenCalledWith(
        'https://accounts.google.test/oauth?state=abc',
      );
    });
  });

  it('shows OAuth callback notices and refreshes Gmail import status', async () => {
    const connectedClient = new ProviderAccountsTestClient();
    connectedClient.syncStatuses = [sampleSyncStatus];
    renderPage({ client: connectedClient, search: '?connected=gmail' });

    expect(await screen.findByRole('status')).toHaveTextContent(
      'Gmail connected. Hail is refreshing import status now.',
    );
    expect(await screen.findByText('Gmail import health')).toBeInTheDocument();

    cleanup();

    renderPage({
      search:
        '?error=oauth_exchange_failed&state=secret-state&code=secret-code',
    });
    expect(await screen.findByRole('alert')).toHaveTextContent(
      'Gmail connection failed while exchanging authorization with Google. Please try again.',
    );
    expect(
      screen.queryByText(/secret-state|secret-code/),
    ).not.toBeInTheDocument();
  });

  it('shows connected Gmail status from sync status and disconnects with confirmation', async () => {
    const client = new ProviderAccountsTestClient();
    client.syncStatuses = [providerSyncStatus({ sync_status: 'active' })];
    const { confirm } = renderPage({ client });

    expect(
      await screen.findAllByText('Reader <reader@gmail.com>'),
    ).toHaveLength(2);
    expect(screen.getAllByText('Connected').length).toBeGreaterThan(0);
    expect(screen.getByText('Gmail account')).toBeInTheDocument();
    expect(screen.getAllByText('12345').length).toBeGreaterThan(0);

    clickButton('Disconnect');

    await waitFor(() => {
      expect(confirm).toHaveBeenCalledWith(
        expect.stringContaining('reader@gmail.com'),
      );
      expect(client.disconnectCalls).toEqual([42]);
      expect(screen.getAllByText('Disconnected').length).toBeGreaterThan(0);
    });
  });

  it('does not disconnect when confirmation is cancelled', async () => {
    const client = new ProviderAccountsTestClient();
    client.syncStatuses = [providerSyncStatus({ sync_status: 'active' })];
    const { confirm } = renderPage({
      client,
      confirm: vi.fn(() => false),
    });

    await screen.findByRole('button', { name: 'Disconnect' });
    clickButton('Disconnect');
    expect(confirm).toHaveBeenCalled();
    expect(client.disconnectCalls).toEqual([]);
  });

  it('shows Connected with green tick when sync_status is active and no error', async () => {
    const client = new ProviderAccountsTestClient();
    client.syncStatuses = [
      providerSyncStatus({
        sync_status: 'active',
        last_error_class: null,
        last_error_message: null,
        last_error_event: null,
      }),
    ];
    renderPage({ client });

    const section = (
      await screen.findByRole('heading', { name: 'Reader <reader@gmail.com>' })
    ).closest('section');
    expect(section).not.toBeNull();
    expect(within(section as HTMLElement).getByText('Connected')).toBeInTheDocument();
  });

  it('shows Connected (recovered) and hides destructive failure copy when last_sync_succeeded_at is after last_error_event.created_at', async () => {
    const client = new ProviderAccountsTestClient();
    client.syncStatuses = [
      providerSyncStatus({
        sync_status: 'error',
        last_sync_attempted_at: '2026-05-26T18:00:00Z',
        last_sync_succeeded_at: '2026-05-26T18:05:00Z',
        last_error_event: {
          event_type: 'history_import',
          result_status: 'failed',
          safe_error_class: 'gmail_rate_limit',
          safe_error_message: 'Gmail asked hail to slow down',
          created_at: '2026-05-26T17:00:00Z',
        },
      }),
    ];
    renderPage({ client });

    const section = (
      await screen.findByRole('heading', { name: 'Reader <reader@gmail.com>' })
    ).closest('section');
    expect(section).not.toBeNull();
    expect(within(section as HTMLElement).getByText('Connected (recovered)')).toBeInTheDocument();
    expect(within(section as HTMLElement).getByText('No active failure — last sync succeeded')).toBeInTheDocument();
    expect(within(section as HTMLElement).getByText(/Show last failure \(recovered/)).toBeInTheDocument();
    expect(within(section as HTMLElement).queryByText('Needs attention')).not.toBeInTheDocument();
    expect(within(section as HTMLElement).queryByText('gmail_rate_limit: Gmail asked hail to slow down')).not.toBeInTheDocument();
  });

  it('shows Syncing Gmail… and disables Sync now while server reports an in-flight attempt', async () => {
    const client = new ProviderAccountsTestClient();
    client.syncStatuses = [
      providerSyncStatus({
        sync_status: 'active',
        last_error_class: null,
        last_error_message: null,
        last_error_event: null,
        last_sync_attempted_at: '2026-05-26T18:00:00Z',
        last_sync_succeeded_at: '2026-05-26T17:00:00Z',
      }),
    ];
    renderPage({ client });

    const section = (
      await screen.findByRole('heading', { name: 'Reader <reader@gmail.com>' })
    ).closest('section');
    expect(section).not.toBeNull();
    expect(within(section as HTMLElement).getByText('Syncing Gmail…')).toBeInTheDocument();
    expect(within(section as HTMLElement).getByRole('button', { name: 'Sync now' })).toBeDisabled();
  });

  it('surfaces client errors for connect and disconnect actions', async () => {
    const connectClient = new ProviderAccountsTestClient();
    connectClient.connectFailure = new HailApiError(
      503,
      undefined,
      response(503),
    );
    renderPage({ client: connectClient });

    clickButton('Connect Gmail');
    expect(
      await screen.findByText('Connect Gmail failed with HTTP 503.'),
    ).toBeInTheDocument();

    cleanup();

    const disconnectClient = new ProviderAccountsTestClient();
    disconnectClient.syncStatuses = [providerSyncStatus({ sync_status: 'active' })];
    disconnectClient.disconnectFailure = new HailApiError(
      500,
      undefined,
      response(500),
    );
    renderPage({ client: disconnectClient });

    await screen.findAllByText('Reader <reader@gmail.com>');
    clickButton('Disconnect');
    expect(
      await screen.findByText('Disconnect Gmail failed with HTTP 500.'),
    ).toBeInTheDocument();
  });

  it('renders only audited safe sync failure event text', async () => {
    const client = new ProviderAccountsTestClient();
    client.syncStatuses = [
      providerSyncStatus({
        last_error_class: 'raw_leak_class',
        last_error_message: 'Bearer raw-token body should not render',
        last_error_event: {
          event_type: 'sync_failed',
          result_status: 'failed',
          safe_error_class: 'gmail_rate_limit',
          safe_error_message: 'Gmail asked hail to slow down',
          created_at: '2026-05-26T17:00:00Z',
        },
      }),
    ];
    renderPage({ client });

    expect(
      await screen.findByText('gmail_rate_limit: Gmail asked hail to slow down'),
    ).toBeInTheDocument();
    expect(screen.queryByText(/raw-token/)).not.toBeInTheDocument();
    expect(screen.queryByText(/raw_leak_class/)).not.toBeInTheDocument();
  });

  it('falls back to safe error class when no safe sync failure message exists', async () => {
    const client = new ProviderAccountsTestClient();
    client.syncStatuses = [
      providerSyncStatus({
        last_error_class: 'gmail_auth_revoked',
        last_error_message: 'Bearer raw-token body should not render',
        last_error_event: null,
        last_sync_succeeded_at: null,
      }),
    ];
    renderPage({ client });

    expect(await screen.findByText('gmail_auth_revoked')).toBeInTheDocument();
    expect(screen.queryByText(/raw-token/)).not.toBeInTheDocument();
  });

  it('renders actionable Stalwart provider quota and rate-limit recovery text', async () => {
    const quotaClient = new ProviderAccountsTestClient();
    quotaClient.syncStatuses = [
      providerSyncStatus({
        last_error_class: 'provider_quota',
        last_error_message: null,
        last_error_event: {
          event_type: 'initial_sync_aborted',
          result_status: 'failed',
          safe_error_class: 'provider_quota',
          safe_error_message: 'Stalwart upload quota exceeded during initial Gmail import',
          created_at: '2026-05-26T17:00:00Z',
        },
        last_sync_succeeded_at: null,
      }),
    ];
    renderPage({ client: quotaClient });

    expect(
      await screen.findByText(
        'Stalwart upload quota exceeded. Increase httpUploadQuota in Stalwart admin → Settings → Network → JMAP → Limits (Files / Size). Then click Re-import.',
      ),
    ).toBeInTheDocument();

    cleanup();

    const rateLimitClient = new ProviderAccountsTestClient();
    rateLimitClient.syncStatuses = [
      providerSyncStatus({
        last_error_class: 'provider_rate_limited',
        last_error_message: null,
        last_error_event: {
          event_type: 'initial_sync_aborted',
          result_status: 'failed',
          safe_error_class: 'provider_rate_limited',
          safe_error_message: 'Stalwart rate limit hit during initial Gmail import',
          created_at: '2026-05-26T17:00:00Z',
        },
        last_sync_succeeded_at: null,
      }),
    ];
    renderPage({ client: rateLimitClient });

    expect(
      await screen.findByText(
        'Stalwart rate limit hit. Increase rateLimitAuthenticated. Then click Re-import.',
      ),
    ).toBeInTheDocument();
  });

  it('shows Gmail sync health and triggers a manual sync', async () => {
    const client = new ProviderAccountsTestClient();
    client.syncStatusResponses = [
      { accounts: [sampleSyncStatus] },
      {
        accounts: [
          providerSyncStatus({
            id: 42,
            sync_status: 'active',
            last_sync_attempted_at: '2026-05-26T18:00:00Z',
            last_sync_succeeded_at: '2026-05-26T18:00:00Z',
            last_error_event: null,
            next_sync_after: null,
            sync_backoff_secs: null,
          }),
        ],
      },
    ];
    client.triggerSyncResponse = {
      account: providerSyncStatus({
        id: 42,
        sync_status: 'active',
        last_sync_attempted_at: '2026-05-26T18:00:00Z',
        last_sync_succeeded_at: '2026-05-26T18:00:00Z',
        last_error_event: null,
        next_sync_after: null,
        sync_backoff_secs: null,
      }),
    };
    renderPage({ client });

    expect(await screen.findByText('Gmail import health')).toBeInTheDocument();
    expect(screen.getAllByText('Needs attention').length).toBeGreaterThan(0);
    expect(screen.getByText('Last successful sync')).toBeInTheDocument();
    expect(screen.getByText('Next retry')).toBeInTheDocument();
    expect(screen.getByText('15 minutes')).toBeInTheDocument();
    expect(
      screen.getByText('gmail_rate_limit: Gmail asked hail to slow down'),
    ).toBeInTheDocument();

    clickButton('Sync now');

    await waitFor(() => {
      expect(client.triggerSyncCalls).toEqual([42]);
      expect(screen.getAllByText('Connected').length).toBeGreaterThan(0);
      expect(screen.getByText('No retry backoff')).toBeInTheDocument();
    });
  });

  it('labels real provider sync statuses from the API enum', async () => {
    const client = new ProviderAccountsTestClient();
    client.syncStatuses = [
      { ...sampleSyncStatus, id: 1, provider_email: 'disabled@gmail.com', display_email: 'Disabled <disabled@gmail.com>', sync_status: 'disabled' },
      { ...sampleSyncStatus, id: 2, provider_email: 'initial@gmail.com', display_email: 'Initial <initial@gmail.com>', sync_status: 'initial_sync' },
      { ...sampleSyncStatus, id: 3, provider_email: 'active@gmail.com', display_email: 'Active <active@gmail.com>', sync_status: 'active' },
      { ...sampleSyncStatus, id: 4, provider_email: 'error@gmail.com', display_email: 'Error <error@gmail.com>', sync_status: 'error' },
      { ...sampleSyncStatus, id: 7, provider_email: 'paused@gmail.com', display_email: 'Paused <paused@gmail.com>', sync_status: 'paused' },
      { ...sampleSyncStatus, id: 5, provider_email: 'revoked@gmail.com', display_email: 'Revoked <revoked@gmail.com>', sync_status: 'revoked' },
      { ...sampleSyncStatus, id: 6, provider_email: 'unknown@gmail.com', display_email: 'Unknown <unknown@gmail.com>', sync_status: 'future_state' },
    ];
    renderPage({ client });

    expect((await screen.findAllByText('Disabled')).length).toBeGreaterThan(0);
    expect(screen.getAllByText('Initial import running').length).toBeGreaterThan(0);
    expect(screen.getAllByText('Connected').length).toBeGreaterThan(0);
    expect(screen.getAllByText('Needs attention').length).toBeGreaterThan(0);
    expect(screen.getAllByText('Paused').length).toBeGreaterThan(0);
    expect(screen.getAllByText('Access revoked').length).toBeGreaterThan(0);
    expect(screen.getAllByText('future_state').length).toBeGreaterThan(0);
  });

  it('shows the re-import button only for connected Gmail accounts', async () => {
    const client = new ProviderAccountsTestClient();
    client.syncStatuses = [
      providerSyncStatus({ id: 1, display_email: 'Active Account', sync_status: 'active' }),
      providerSyncStatus({ id: 2, display_email: 'Error Account', sync_status: 'error' }),
      providerSyncStatus({ id: 3, display_email: 'Initial Account', sync_status: 'initial_sync' }),
      providerSyncStatus({ id: 7, display_email: 'Paused Account', sync_status: 'paused' }),
      providerSyncStatus({ id: 4, display_email: 'Disabled Account', sync_status: 'disabled' }),
      providerSyncStatus({ id: 5, display_email: 'Revoked Account', sync_status: 'revoked' }),
      providerSyncStatus({ id: 6, display_email: 'Disconnected Account', sync_status: 'disconnected' }),
    ];
    renderPage({ client });

    expect(
      within((await screen.findByRole('heading', { name: 'Active Account' })).closest('section') as HTMLElement)
        .getByRole('button', { name: 'Re-import from Gmail' }),
    ).toBeEnabled();
    expect(
      within((await screen.findByRole('heading', { name: 'Error Account' })).closest('section') as HTMLElement)
        .getByRole('button', { name: 'Re-import from Gmail' }),
    ).toBeEnabled();
    expect(
      within((await screen.findByRole('heading', { name: 'Re-importing from Gmail…' })).closest('section') as HTMLElement)
        .getByRole('button', { name: 'Re-import from Gmail' }),
    ).toBeDisabled();
    expect(
      within((await screen.findByRole('heading', { name: 'Paused Account' })).closest('section') as HTMLElement)
        .getByRole('button', { name: 'Re-import from Gmail' }),
    ).toBeEnabled();

    for (const accountName of ['Disabled Account', 'Revoked Account', 'Disconnected Account']) {
      expect(
        within((await screen.findByRole('heading', { name: accountName })).closest('section') as HTMLElement)
          .queryByRole('button', { name: 'Re-import from Gmail' }),
      ).not.toBeInTheDocument();
    }
  });

  it('opens the stop import confirmation dialog; cancel does nothing; confirm pauses import', async () => {
    const client = new ProviderAccountsTestClient();
    client.syncStatuses = [providerSyncStatus({
      sync_status: 'active',
      last_sync_attempted_at: '2026-05-26T18:00:00Z',
      last_sync_succeeded_at: '2026-05-26T17:00:00Z',
    })];
    renderPage({ client });

    fireEvent.click(await screen.findByRole('button', { name: 'Stop import' }));
    expect(await screen.findByRole('alertdialog')).toHaveTextContent('Stop Gmail import?');
    expect(screen.getByText(/The current sync will pause after the in-flight batch/)).toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));
    await waitFor(() => expect(screen.queryByRole('alertdialog')).not.toBeInTheDocument());
    expect(client.stopCalls).toEqual([]);

    fireEvent.click(screen.getByRole('button', { name: 'Stop import' }));
    fireEvent.click(await screen.findByRole('button', { name: 'Stop import' }));

    await waitFor(() => {
      expect(client.stopCalls).toEqual([42]);
      expect(screen.getAllByText('Paused').length).toBeGreaterThan(0);
      expect(screen.getByText('Paused — click Sync now to resume')).toBeInTheDocument();
      expect(screen.getByRole('button', { name: 'Sync now' })).toBeEnabled();
      expect(screen.getByRole('button', { name: 'Re-import from Gmail' })).toBeEnabled();
    });
  });

  it('shows the stop import button only during server sync or initial import', async () => {
    const client = new ProviderAccountsTestClient();
    client.syncStatuses = [
      providerSyncStatus({ id: 1, display_email: 'Active Account', sync_status: 'active' }),
      providerSyncStatus({ id: 2, display_email: 'Syncing Account', sync_status: 'active', last_sync_attempted_at: '2026-05-26T18:00:00Z', last_sync_succeeded_at: '2026-05-26T17:00:00Z' }),
      providerSyncStatus({ id: 3, display_email: 'Initial Account', sync_status: 'initial_sync' }),
      providerSyncStatus({ id: 4, display_email: 'Paused Account', sync_status: 'paused' }),
    ];
    renderPage({ client });

    expect(
      within((await screen.findByRole('heading', { name: 'Active Account' })).closest('section') as HTMLElement)
        .queryByRole('button', { name: 'Stop import' }),
    ).not.toBeInTheDocument();
    expect(
      within((await screen.findByRole('heading', { name: 'Syncing Account' })).closest('section') as HTMLElement)
        .getByRole('button', { name: 'Stop import' }),
    ).toBeEnabled();
    expect(
      within((await screen.findByRole('heading', { name: 'Re-importing from Gmail…' })).closest('section') as HTMLElement)
        .getByRole('button', { name: 'Stop import' }),
    ).toBeEnabled();
    expect(
      within((await screen.findByRole('heading', { name: 'Paused Account' })).closest('section') as HTMLElement)
        .queryByRole('button', { name: 'Stop import' }),
    ).not.toBeInTheDocument();
  });

  it('opens the re-import confirmation dialog and cancel does nothing', async () => {
    const client = new ProviderAccountsTestClient();
    client.syncStatuses = [providerSyncStatus({ sync_status: 'active' })];
    renderPage({ client });

    fireEvent.click(await screen.findByRole('button', { name: 'Re-import from Gmail' }));
    expect(await screen.findByRole('alertdialog')).toHaveTextContent('Re-import from Gmail?');
    expect(screen.getByText(/Hail will re-fetch all Gmail messages from scratch/)).toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));
    await waitFor(() => expect(screen.queryByRole('alertdialog')).not.toBeInTheDocument());
    expect(client.reimportCalls).toEqual([]);
  });

  it('confirms re-import, flips to initial sync, disables actions, and updates the header', async () => {
    const client = new ProviderAccountsTestClient();
    client.syncStatuses = [providerSyncStatus({ sync_status: 'active' })];
    renderPage({ client });

    fireEvent.click(await screen.findByRole('button', { name: 'Re-import from Gmail' }));
    fireEvent.click(await screen.findByRole('button', { name: 'Re-import' }));

    await waitFor(() => {
      expect(client.reimportCalls).toEqual([42]);
      expect(screen.getByRole('heading', { name: 'Re-importing from Gmail…' })).toBeInTheDocument();
      expect(screen.getByRole('button', { name: 'Sync now' })).toBeDisabled();
      expect(screen.getByRole('button', { name: 'Re-import from Gmail' })).toBeDisabled();
    });
  });

  it('surfaces sync status and manual sync errors', async () => {
    const statusClient = new ProviderAccountsTestClient();
    statusClient.syncStatusFailure = new HailApiError(
      503,
      undefined,
      response(503),
    );
    renderPage({ client: statusClient });

    expect(
      await screen.findByText('Load Gmail import status failed with HTTP 503.'),
    ).toBeInTheDocument();

    cleanup();

    const syncClient = new ProviderAccountsTestClient();
    syncClient.syncStatuses = [sampleSyncStatus];
    syncClient.triggerSyncFailure = new HailApiError(
      500,
      undefined,
      response(500),
    );
    renderPage({ client: syncClient });

    await screen.findByText('Gmail import health');
    clickButton('Sync now');
    expect(
      await screen.findByText('Sync Gmail now failed with HTTP 500.'),
    ).toBeInTheDocument();
  });

  it('renders canonical provider sync statuses with realistic labels', async () => {
    const client = new ProviderAccountsTestClient();
    client.syncStatuses = [
      providerSyncStatus({
        id: 1,
        display_email: 'Initial Account',
        sync_status: 'initial_sync',
      }),
      providerSyncStatus({
        id: 2,
        display_email: 'Active Account',
        sync_status: 'active',
      }),
      providerSyncStatus({
        id: 3,
        display_email: 'Error Account',
        sync_status: 'error',
      }),
      providerSyncStatus({
        id: 4,
        display_email: 'Disabled Account',
        sync_status: 'disabled',
      }),
      providerSyncStatus({
        id: 5,
        display_email: 'Revoked Account',
        sync_status: 'revoked',
      }),
      providerSyncStatus({
        id: 6,
        display_email: 'Paused Account',
        sync_status: 'paused',
      }),
    ];

    renderPage({ client });

    for (const [accountName, label] of [
      ['Re-importing from Gmail…', 'Initial import running'],
      ['Active Account', 'Connected'],
      ['Error Account', 'Needs attention'],
      ['Disabled Account', 'Disabled'],
      ['Revoked Account', 'Access revoked'],
      ['Paused Account', 'Paused'],
    ]) {
      const section = (
        await screen.findByRole('heading', { name: accountName })
      ).closest('section');
      expect(section).not.toBeNull();
      expect(
        within(section as HTMLElement).getByText(label),
      ).toBeInTheDocument();
    }
  });

  it('disables manual sync for disconnected, disabled, and revoked status cards', async () => {
    const client = new ProviderAccountsTestClient();
    client.syncStatuses = [
      providerSyncStatus({
        id: 1,
        display_email: 'Active Account',
        sync_status: 'active',
      }),
      providerSyncStatus({
        id: 2,
        display_email: 'Disabled Account',
        sync_status: 'disabled',
      }),
      providerSyncStatus({
        id: 3,
        display_email: 'Revoked Account',
        sync_status: 'revoked',
      }),
      providerSyncStatus({
        id: 4,
        display_email: 'Disconnected Account',
        sync_status: 'disconnected',
      }),
    ];

    renderPage({ client });

    expect(
      within(
        (
          await screen.findByRole('heading', { name: 'Active Account' })
        ).closest('section') as HTMLElement,
      ).getByRole('button', { name: 'Sync now' }),
    ).toBeEnabled();
    for (const accountName of [
      'Disabled Account',
      'Revoked Account',
      'Disconnected Account',
    ]) {
      const button = within(
        (await screen.findByRole('heading', { name: accountName })).closest(
          'section',
        ) as HTMLElement,
      ).getByRole('button', { name: 'Sync now' });
      expect(button).toBeDisabled();
      fireEvent.click(button);
    }
    expect(client.triggerSyncCalls).toEqual([]);
  });

  it('prevents duplicate manual sync requests while one is pending', async () => {
    const client = new ProviderAccountsTestClient();
    const pendingSync = deferred<ProviderSyncTriggerResponse>();
    client.syncStatuses = [sampleSyncStatus];
    client.triggerSyncPromise = pendingSync.promise;
    renderPage({ client });

    fireEvent.click(await screen.findByRole('button', { name: 'Sync now' }));
    await waitFor(() =>
      expect(
        screen.getByRole('button', { name: 'Requesting sync…' }),
      ).toBeDisabled(),
    );
    fireEvent.click(screen.getByRole('button', { name: 'Requesting sync…' }));
    expect(client.triggerSyncCalls).toEqual([42]);

    pendingSync.resolve({
      account: providerSyncStatus({ id: 42, sync_status: 'active' }),
    });
    await waitFor(() =>
      expect(screen.getByRole('button', { name: 'Sync now' })).toBeEnabled(),
    );
    expect(client.triggerSyncCalls).toEqual([42]);
  });

  it('updates cached sync status from manual sync success and then invalidates it', async () => {
    const client = new ProviderAccountsTestClient();
    const queryClient = createTestQueryClient();
    const invalidateSpy = vi.spyOn(queryClient, 'invalidateQueries');
    const activeStatus = providerSyncStatus({
      id: 42,
      sync_status: 'active',
      next_sync_after: null,
      sync_backoff_secs: null,
      last_error_class: null,
      last_error_message: null,
      last_error_event: null,
    });
    client.syncStatusResponses = [
      { accounts: [sampleSyncStatus] },
      { accounts: [activeStatus] },
    ];
    client.triggerSyncResponse = { account: activeStatus };
    renderPage({ client, queryClient });

    fireEvent.click(await screen.findByRole('button', { name: 'Sync now' }));

    await waitFor(() => {
      const cached = queryClient.getQueryData<ProviderSyncStatusListResponse>(
        queryKeys.providerSyncStatuses(),
      );
      expect(cached?.accounts[0]?.sync_status).toBe('active');
      expect(screen.getAllByText('Connected').length).toBeGreaterThan(0);
    });
    expect(invalidateSpy).toHaveBeenCalledWith({
      queryKey: queryKeys.providerSyncStatuses(),
    });
  });

  it('uses provider sync status as the only persistent account card source', async () => {
    const client = new ProviderAccountsTestClient();
    client.syncStatuses = [sampleSyncStatus];
    renderPage({ client, account: providerSyncStatus({ id: 99, provider_email: 'stale@gmail.com', display_email: 'Stale Account' }) });

    expect(await screen.findByText('Gmail import health')).toBeInTheDocument();
    expect(screen.getAllByText('Gmail account')).toHaveLength(1);
    expect(screen.getAllByText('Reader <reader@gmail.com>')).toHaveLength(2);
    expect(screen.queryByText('Stale Account')).not.toBeInTheDocument();
  });

  it('updates cached sync status after disconnect and invalidates it', async () => {
    const client = new ProviderAccountsTestClient();
    const queryClient = createTestQueryClient();
    const invalidateSpy = vi.spyOn(queryClient, 'invalidateQueries');
    client.syncStatuses = [providerSyncStatus({ sync_status: 'active' })];
    client.disconnectResponse = providerAccount({
      sync_status: 'disconnected',
      cached_access_token_expires_at: null,
    });
    renderPage({ client, queryClient });

    fireEvent.click(await screen.findByRole('button', { name: 'Disconnect' }));

    await waitFor(() => {
      const cached = queryClient.getQueryData<ProviderSyncStatusListResponse>(
        queryKeys.providerSyncStatuses(),
      );
      expect(cached?.accounts[0]?.sync_status).toBe('disconnected');
      expect(screen.getAllByText('Disconnected').length).toBeGreaterThan(0);
    });
    expect(invalidateSpy).toHaveBeenCalledWith({
      queryKey: queryKeys.providerSyncStatuses(),
    });
  });

  it('disconnects without duplicate stale account cards', async () => {
    const client = new ProviderAccountsTestClient();
    client.syncStatuses = [sampleSyncStatus];
    client.disconnectResponse = providerAccount({
      sync_status: 'disconnected',
      cached_access_token_expires_at: null,
    });
    renderPage({ client });

    expect(await screen.findByText('Gmail import health')).toBeInTheDocument();
    expect(screen.getByText('Gmail account')).toBeInTheDocument();

    clickButton('Disconnect');

    await waitFor(() => {
      expect(client.disconnectCalls).toEqual([42]);
      expect(screen.getByText('Gmail import health')).toBeInTheDocument();
      expect(screen.getAllByText('Gmail account')).toHaveLength(1);
      expect(
        screen.getByRole('button', { name: 'Disconnected' }),
      ).toBeDisabled();
    });
  });

  it('uses the real API client fetch path for provider account actions', async () => {
    const fetchSpy = vi.spyOn(globalThis, 'fetch').mockImplementation((input) => {
      const url = input instanceof Request ? input.url : String(input);
      if (url.endsWith('/api/provider-accounts/gmail/connect')) {
        return Promise.resolve(
          jsonResponse(200, {
            authorization_url:
              'https://accounts.google.test/oauth?state=from-fetch',
            scopes: ['https://www.googleapis.com/auth/gmail.readonly'],
          }),
        );
      }
      if (url.endsWith('/api/provider-accounts/sync-status')) {
        return Promise.resolve(jsonResponse(200, { accounts: [] }));
      }
      return Promise.resolve(jsonResponse(200, { items: [] }));
    });
    const assign = vi.fn();

    renderPage({
      client: new HailApiClient({ baseUrl: 'http://localhost' }),
      assign,
    });

    const connectButton = await screen.findByRole('button', { name: 'Connect Gmail' });
    fireEvent.click(connectButton);

    await waitFor(() =>
      expect(assign).toHaveBeenCalledWith(
        'https://accounts.google.test/oauth?state=from-fetch',
      ),
    );
    const statusCall = fetchSpy.mock.calls.find(([url]) =>
      String(url).endsWith('/api/provider-accounts/sync-status'),
    );
    expect(statusCall?.[0]).toEqual(
      new URL('http://localhost/api/provider-accounts/sync-status'),
    );
    const connectCall = fetchSpy.mock.calls.find(([url]) =>
      String(url).endsWith('/api/provider-accounts/gmail/connect'),
    );
    expect(connectCall?.[0]).toEqual(
      new URL('http://localhost/api/provider-accounts/gmail/connect'),
    );
    expect(connectCall?.[1]).toMatchObject({
      method: 'POST',
      credentials: 'include',
    });
    expect(
      new Headers(connectCall?.[1]?.headers).get('X-Hail-Request'),
    ).toBe('1');
  });
});
