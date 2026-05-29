import { RouterProvider } from '@tanstack/react-router';
import { cleanup, fireEvent, screen, waitFor } from '@testing-library/react';
import type { ReactNode } from 'react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type {
  AddAdminDomainRequest,
  AdminDomainResponse,
  AdminDomainsResponse,
  AdminStatsResponse,
  AdminUsersResponse,
  CreateInviteRequest,
  CreatedInviteEnvelope,
  LabelListResponse,
  ResetAdminUserPasswordRequest,
  UserEnvelope,
  UserView,
  ViewCountsResponse,
} from '../api/client';
import { HailApiError } from '../api/client';
import { defaultApiClient } from '../api/query';
import { AuthProvider } from '../auth/AuthProvider';
import { router } from '../router';
import {
  createTestQueryClient,
  renderWithQueryClient,
  seedMe,
  TestHailApiClient,
} from '../test-utils';
import { AdminPage } from './AdminPage';

const currentAdmin: UserView = {
  id: 1,
  email: 'admin@example.com',
  display_name: 'Admin Reader',
  is_admin: true,
};

const teammate: UserView = {
  id: 2,
  email: 'teammate@example.com',
  display_name: 'Team Mate',
  is_admin: false,
};

function adminEnvelope(user: UserView = currentAdmin): UserEnvelope {
  return { user };
}

function response(status: number) {
  return new Response(JSON.stringify({ error: 'boom' }), {
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

class AdminPageTestClient extends TestHailApiClient {
  users: UserView[] = [currentAdmin, teammate];
  domains: string[] = ['example.com'];
  statsResponses: AdminStatsResponse[] = [];
  statsPromise: Promise<AdminStatsResponse> | null = null;
  createInviteCalls: CreateInviteRequest[] = [];
  resetPasswordCalls: Array<{ userId: number; body: ResetAdminUserPasswordRequest }> = [];
  addDomainCalls: AddAdminDomainRequest[] = [];
  deleteDomainCalls: string[] = [];
  deleteUserCalls: number[] = [];
  statsCalls = 0;
  createInviteFailure: Error | null = null;

  constructor(testUser: UserEnvelope = adminEnvelope()) {
    super(testUser);
  }

  override async getViewCounts(): Promise<ViewCountsResponse> {
    return {
      imbox_new: 0,
      feed_unread: 0,
      papertrail_unread: 0,
      screener_pending: 0,
      drafts: 0,
      scheduled: 0,
      set_aside: 0,
      reply_later: 0,
      bubble_up: 0,
      spam: 0,
      trash: 0,
    };
  }

  override async listLabels(): Promise<LabelListResponse> {
    return { labels: [] };
  }

  override async listAdminUsers(): Promise<AdminUsersResponse> {
    return { users: this.users };
  }

  override async getAdminStats(): Promise<AdminStatsResponse> {
    this.statsCalls += 1;
    if (this.statsPromise) return this.statsPromise;
    return this.statsResponses.shift() ?? {
      stalwart_status: 'connected',
      users: this.users.map((user, index) => ({
        email: user.email,
        mailbox_count: 1,
        total_emails: index === 0 ? 12 : 5,
        total_size_bytes: index === 0 ? 2048 : 1024,
      })),
    };
  }

  override async createInvite(body: CreateInviteRequest): Promise<CreatedInviteEnvelope> {
    this.createInviteCalls.push(body);
    if (this.createInviteFailure) throw this.createInviteFailure;
    return {
      invite: {
        email: body.email,
        display_name: body.display_name,
        expires_at: '2026-05-30T12:00:00Z',
        invite_url: `https://hail.test/invite/${encodeURIComponent(body.email)}`,
      },
    };
  }

  override async resetAdminUserPassword(
    userId: number,
    body: ResetAdminUserPasswordRequest,
  ): Promise<UserEnvelope> {
    this.resetPasswordCalls.push({ userId, body });
    return { user: this.users.find((user) => user.id === userId) ?? teammate };
  }

  override async listAdminDomains(): Promise<AdminDomainsResponse> {
    return { domains: this.domains };
  }

  override async addAdminDomain(body: AddAdminDomainRequest): Promise<AdminDomainResponse> {
    this.addDomainCalls.push(body);
    if (!this.domains.includes(body.domain)) {
      this.domains = [...this.domains, body.domain];
    }
    return { domain: body.domain };
  }

  override async deleteAdminDomain(domain: string): Promise<void> {
    this.deleteDomainCalls.push(domain);
    this.domains = this.domains.filter((current) => current !== domain);
  }

  override async deleteAdminUser(userId: number): Promise<void> {
    this.deleteUserCalls.push(userId);
    this.users = this.users.filter((user) => user.id !== userId);
  }
}

let currentTestBody: ReactNode = null;
let restoreAdminRoute: (() => void) | null = null;

function TestBody() {
  return currentTestBody;
}

function installTestRouteComponent() {
  if (restoreAdminRoute) return;
  const matchRoute = router.routesByPath['/admin'];
  const previousComponent = matchRoute.options.component;
  const previousBeforeLoad = matchRoute.options.beforeLoad;
  matchRoute.options.component = TestBody;
  matchRoute.options.beforeLoad = undefined;
  restoreAdminRoute = () => {
    matchRoute.options.component = previousComponent;
    matchRoute.options.beforeLoad = previousBeforeLoad;
  };
}

function restoreRoute() {
  restoreAdminRoute?.();
  restoreAdminRoute = null;
}

function installDefaultClientSpies(client: AdminPageTestClient) {
  vi.spyOn(defaultApiClient, 'me').mockImplementation(() => client.me());
  vi.spyOn(defaultApiClient, 'getViewCounts').mockImplementation(() => client.getViewCounts());
  vi.spyOn(defaultApiClient, 'listLabels').mockImplementation(() => client.listLabels());
  vi.spyOn(defaultApiClient, 'listAdminUsers').mockImplementation(() => client.listAdminUsers());
  vi.spyOn(defaultApiClient, 'getAdminStats').mockImplementation(() => client.getAdminStats());
  vi.spyOn(defaultApiClient, 'createInvite').mockImplementation((body) => client.createInvite(body));
  vi.spyOn(defaultApiClient, 'resetAdminUserPassword').mockImplementation((userId, body) =>
    client.resetAdminUserPassword(userId, body),
  );
  vi.spyOn(defaultApiClient, 'listAdminDomains').mockImplementation(() => client.listAdminDomains());
  vi.spyOn(defaultApiClient, 'addAdminDomain').mockImplementation((body) => client.addAdminDomain(body));
  vi.spyOn(defaultApiClient, 'deleteAdminDomain').mockImplementation((domain) => client.deleteAdminDomain(domain));
  vi.spyOn(defaultApiClient, 'deleteAdminUser').mockImplementation((userId) => client.deleteAdminUser(userId));
}

function renderAdminPage(client = new AdminPageTestClient()) {
  installDefaultClientSpies(client);
  const queryClient = createTestQueryClient();
  seedMe(queryClient, client.testUser);
  currentTestBody = (
    <AuthProvider>
      <AdminPage />
    </AuthProvider>
  );
  installTestRouteComponent();
  window.history.pushState({}, '', '/admin');
  renderWithQueryClient(<RouterProvider router={router} />, queryClient);
  return { client };
}

function submitInvite(email: string, displayName: string) {
  fireEvent.change(screen.getByLabelText('Email'), { target: { value: email } });
  fireEvent.change(screen.getByLabelText('Display name'), { target: { value: displayName } });
  fireEvent.click(screen.getByRole('button', { name: 'Create invite link' }));
}

afterEach(() => {
  currentTestBody = null;
  restoreRoute();
  window.history.pushState({}, '', '/');
  cleanup();
  vi.restoreAllMocks();
});

describe('AdminPage', () => {
  it('creates an invite link with the submitted payload and renders a selectable URL', async () => {
    const { client } = renderAdminPage();
    expect(await screen.findByRole('heading', { name: 'Invite user' })).toBeInTheDocument();

    submitInvite('new.person@example.com', 'New Person');

    const inviteLink = await screen.findByLabelText('Invite link');
    expect(client.createInviteCalls).toEqual([
      { email: 'new.person@example.com', display_name: 'New Person' },
    ]);
    expect(inviteLink).toHaveValue('https://hail.test/invite/new.person%40example.com');
    expect(inviteLink).toHaveAttribute('readonly');

    fireEvent.focus(inviteLink);
    expect((inviteLink as HTMLInputElement).selectionStart).toBe(0);
    expect((inviteLink as HTMLInputElement).selectionEnd).toBe(
      'https://hail.test/invite/new.person%40example.com'.length,
    );
  });

  it('shows an invite creation error without rendering a stale invite URL', async () => {
    const client = new AdminPageTestClient();
    client.createInviteFailure = new HailApiError(503, undefined, response(503));
    renderAdminPage(client);

    expect(await screen.findByRole('heading', { name: 'Invite user' })).toBeInTheDocument();
    submitInvite('new.person@example.com', 'New Person');

    expect(await screen.findByText('Create invite failed with HTTP 503.')).toBeInTheDocument();
    expect(screen.queryByLabelText('Invite link')).not.toBeInTheDocument();
    expect(client.createInviteCalls).toEqual([
      { email: 'new.person@example.com', display_name: 'New Person' },
    ]);
  });

  it('refreshes stats through loading, empty, and result states', async () => {
    const client = new AdminPageTestClient();
    client.users = [];
    client.domains = [];
    const pendingStats = deferred<AdminStatsResponse>();
    client.statsPromise = pendingStats.promise;
    renderAdminPage(client);

    expect(await screen.findAllByText('…')).toHaveLength(3);
    expect(await screen.findByText('No users')).toBeInTheDocument();
    expect(screen.getByText('No domains configured yet.')).toBeInTheDocument();

    pendingStats.resolve({ stalwart_status: 'connected', users: [] });
    await waitFor(() => {
      expect(screen.getAllByText('0').length).toBeGreaterThan(0);
      expect(screen.getByText('0 B')).toBeInTheDocument();
    });

    const refreshStats = deferred<AdminStatsResponse>();
    client.statsPromise = refreshStats.promise;
    fireEvent.click(screen.getByRole('button', { name: 'Refresh stats' }));

    await waitFor(() => expect(screen.getByRole('button', { name: 'Refreshing…' })).toBeDisabled());
    client.statsPromise = null;
    refreshStats.resolve({
      stalwart_status: 'connected',
      users: [{ email: 'teammate@example.com', mailbox_count: 1, total_emails: 42, total_size_bytes: 1536 }],
    });
    await waitFor(() => {
      expect(screen.getByText('42')).toBeInTheDocument();
      expect(screen.getByText('1.50 KB')).toBeInTheDocument();
    });
    expect(client.statsCalls).toBeGreaterThanOrEqual(2);
  });

  it('resets a non-self user password and shows success', async () => {
    const { client } = renderAdminPage();
    await screen.findByRole('heading', { name: 'teammate@example.com' });

    const passwordFields = screen.getAllByLabelText('Reset password');
    fireEvent.change(passwordFields[1], { target: { value: 'correct-horse-battery' } });
    fireEvent.click(screen.getAllByRole('button', { name: 'Reset' })[1]);

    expect(await screen.findByRole('status')).toHaveTextContent('Password reset for teammate@example.com.');
    expect(client.resetPasswordCalls).toEqual([
      { userId: 2, body: { password: 'correct-horse-battery' } },
    ]);
    expect(passwordFields[1]).toHaveValue('');
  });

  it('adds and deletes domains, while cancellation blocks domain deletion', async () => {
    const confirm = vi.spyOn(window, 'confirm').mockReturnValue(false);
    const { client } = renderAdminPage();

    expect(await screen.findByText('example.com')).toBeInTheDocument();
    fireEvent.change(screen.getByLabelText('Domain'), { target: { value: 'new.example' } });
    fireEvent.click(screen.getByRole('button', { name: 'Add domain' }));

    await waitFor(() => {
      expect(client.addDomainCalls).toEqual([{ domain: 'new.example' }]);
      expect(screen.getByText('new.example')).toBeInTheDocument();
    });

    fireEvent.click(screen.getByRole('button', { name: 'Delete domain example.com' }));
    expect(confirm).toHaveBeenCalledWith(expect.stringContaining('example.com'));
    expect(client.deleteDomainCalls).toEqual([]);
    expect(screen.getByText('example.com')).toBeInTheDocument();

    confirm.mockReturnValue(true);
    fireEvent.click(screen.getByRole('button', { name: 'Delete domain example.com' }));

    await waitFor(() => {
      expect(client.deleteDomainCalls).toEqual(['example.com']);
      expect(screen.queryByText('example.com')).not.toBeInTheDocument();
    });
  });

  it('prevents self deletion and deletes another user', async () => {
    const confirm = vi.spyOn(window, 'confirm').mockReturnValue(true);
    const { client } = renderAdminPage();

    expect(await screen.findByRole('heading', { name: 'admin@example.com' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Delete user admin@example.com' })).toBeDisabled();

    fireEvent.click(screen.getByRole('button', { name: 'Delete user teammate@example.com' }));

    await waitFor(() => {
      expect(confirm).toHaveBeenCalledWith(expect.stringContaining('teammate@example.com'));
      expect(client.deleteUserCalls).toEqual([2]);
      expect(screen.queryByRole('heading', { name: 'teammate@example.com' })).not.toBeInTheDocument();
    });
  });
});
