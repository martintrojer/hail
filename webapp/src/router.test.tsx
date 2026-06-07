import { QueryClientProvider } from '@tanstack/react-query';
import { RouterProvider } from '@tanstack/react-router';
import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type {
  LoginRequest,
  MailViewResponse,
  ScreenerView,
  SetupAdminRequest,
  SetupState,
  UserEnvelope,
} from './api/client';
import { queryKeys } from './api/queryKeys';
import { queryClient } from './lib/queryClient';
import { router } from './router';

vi.mock('./api/events', () => ({
  useHailEvents: vi.fn(),
}));

const adminUser: UserEnvelope = {
  user: {
    id: 1,
    email: 'admin@example.com',
    display_name: 'Admin User',
    is_admin: true,
  },
};

const imbox: MailViewResponse = {
  items: [],
  next_cursor: null,
};

const screenerView: ScreenerView = {
  senders: [
    {
      sender: 'new@example.com',
      message_count: 1,
      emails: [],
      latest_preview: null,
      first_seen_at: '2026-05-27T12:00:00Z',
    },
    {
      sender: 'another@example.com',
      message_count: 1,
      emails: [],
      latest_preview: null,
      first_seen_at: '2026-05-27T12:05:00Z',
    },
  ],
};

function jsonResponse(body: unknown, status = 200) {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

interface ApiState {
  setup: SetupState;
  user: UserEnvelope | null;
  loginCalls: LoginRequest[];
  setupAdminCalls: SetupAdminRequest[];
  setupAdminError?: { status: number; body: unknown };
}

let api: ApiState;
let fetchMock: ReturnType<typeof vi.fn>;

beforeEach(() => {
  api = {
    setup: { wizard_active: false, backend: 'jmap', reason: 'admin_user_exists' },
    user: null,
    loginCalls: [],
    setupAdminCalls: [],
  };
  queryClient.clear();
  queryClient.setDefaultOptions({
    queries: { retry: false, staleTime: 0, gcTime: Infinity },
    mutations: { retry: false },
  });
  fetchMock = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = new URL(input.toString(), 'http://localhost');
    const method = init?.method ?? 'GET';

    if (url.pathname === '/api/setup/state' && method === 'GET') {
      return jsonResponse(api.setup);
    }

    if (url.pathname === '/api/auth/me' && method === 'GET') {
      if (api.user === null) {
        return jsonResponse({ error: 'unauthenticated' }, 401);
      }
      return jsonResponse(api.user);
    }

    if (url.pathname === '/api/auth/login' && method === 'POST') {
      const body = JSON.parse(init?.body as string) as LoginRequest;
      api.loginCalls.push(body);
      api.user = adminUser;
      return jsonResponse(adminUser);
    }

    if (url.pathname === '/api/setup/admin' && method === 'POST') {
      const body = JSON.parse(init?.body as string) as SetupAdminRequest;
      api.setupAdminCalls.push(body);
      if (api.setupAdminError) {
        return jsonResponse(api.setupAdminError.body, api.setupAdminError.status);
      }
      api.user = adminUser;
      api.setup = { wizard_active: false, backend: 'jmap', reason: 'admin_user_exists' };
      return jsonResponse(adminUser, 201);
    }

    if (url.pathname === '/api/views/imbox/sectioned' && method === 'GET') {
      return jsonResponse({
        bubbled_up: [],
        new_for_you: imbox.items,
        previously_seen: [],
        new_count: 0,
        previously_seen_total: 0,
        next_cursor: null,
      });
    }

    if (url.pathname === '/api/views/counts' && method === 'GET') {
      return jsonResponse({
        imbox_new: 4,
        feed_unread: 3,
        papertrail_unread: 2,
        screener_pending: 2,
        drafts: 1,
        scheduled: 5,
        set_aside: 6,
        reply_later: 7,
        bubble_up: 8,
        spam: 9,
        trash: 10,
      });
    }

    if (url.pathname === '/api/views/imbox' && method === 'GET') {
      return jsonResponse(imbox);
    }

    if (url.pathname === '/api/labels' && method === 'GET') {
      return jsonResponse({
        labels: [
          {
            id: 41,
            name: 'Family',
            leaf_name: 'Family',
            path_segments: ['Family'],
            source: 'manual',
            color: null,
            thread_count: 1,
          },
          {
            id: 40,
            name: 'Work',
            leaf_name: 'Work',
            path_segments: ['Work'],
            source: 'manual',
            color: null,
            thread_count: 3,
          },
          {
            id: 42,
            name: 'Work/Receipts',
            leaf_name: 'Receipts',
            path_segments: ['Work', 'Receipts'],
            source: 'manual',
            color: null,
            thread_count: 0,
          },
        ],
      });
    }

    if (url.pathname === '/api/labels/42/threads' && method === 'GET') {
      return jsonResponse({
        label: {
          id: 42,
          name: 'Work/Receipts',
          leaf_name: 'Receipts',
          path_segments: ['Work', 'Receipts'],
          source: 'manual',
          color: null,
          thread_count: 0,
        },
        items: [],
        next_cursor: null,
      });
    }

    if (url.pathname === '/api/views/screener' && method === 'GET') {
      return jsonResponse(screenerView);
    }

    return jsonResponse({ error: `unhandled ${method} ${url.pathname}` }, 500);
  });
  vi.stubGlobal('fetch', fetchMock);
  window.history.pushState({}, '', '/');
});

afterEach(() => {
  cleanup();
  queryClient.clear();
  vi.unstubAllGlobals();
  window.history.pushState({}, '', '/');
});

function renderRouterAt(path: string) {
  window.history.pushState({}, '', path);
  render(
    <QueryClientProvider client={queryClient}>
      <RouterProvider router={router} />
    </QueryClientProvider>,
  );
}

async function expectLocation(path: string) {
  await waitFor(() => {
    expect(window.location.pathname).toBe(path);
  });
}

describe('SPA auth/router flows', () => {
  it('redirects the root route to setup when setup is active', async () => {
    api.setup = { wizard_active: true, backend: 'jmap' };

    renderRouterAt('/');

    await expectLocation('/setup');
    expect(await screen.findByRole('heading', { name: 'First-run setup' })).toBeInTheDocument();
    expect(
      fetchMock.mock.calls.some(
        ([url, init]) =>
          String(url) === `${window.location.origin}/api/setup/state` &&
          (init as RequestInit | undefined)?.credentials === 'include',
      ),
    ).toBe(true);
  });

  it('redirects unauthenticated protected routes to login', async () => {
    renderRouterAt('/imbox');

    await expectLocation('/login');
    expect(await screen.findByRole('heading', { name: 'Sign in' })).toBeInTheDocument();
    expect(
      fetchMock.mock.calls.some(
        ([url, init]) =>
          String(url) === `${window.location.origin}/api/auth/me` &&
          (init as RequestInit | undefined)?.credentials === 'include',
      ),
    ).toBe(true);
  });

  it('lets authenticated users reach imbox', async () => {
    api.user = adminUser;

    renderRouterAt('/imbox');

    await expectLocation('/imbox');
    expect(await screen.findByRole('heading', { name: 'Imbox' })).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: /hail/i }));
    expect(await screen.findByText('admin@example.com')).toBeInTheDocument();
  });

  it('shows sidebar counter pills from the nav count endpoint', async () => {
    api.user = adminUser;

    renderRouterAt('/imbox');

    for (const [name, count] of [
      ['Imbox', 4],
      ['The Feed', 3],
      ['Paper Trail', 2],
      ['The Screener', 2],
      ['Drafts', 1],
      ['Scheduled', 5],
      ['Set Aside', 6],
      ['Reply Later', 7],
      ['Bubble Up', 8],
      ['Spam', 9],
      ['Trash', 10],
    ] as const) {
      expect(
        await screen.findByRole('link', { name: `${name}, ${count} items` }),
      ).toBeInTheDocument();
    }
  });

  it('links to the Speakeasy passphrase bypass page from the sidebar tools', async () => {
    api.user = adminUser;

    renderRouterAt('/imbox');

    const speakeasyLink = await screen.findByRole('link', {
      name: 'Speakeasy Passphrase',
    });
    expect(speakeasyLink).toHaveAttribute('href', '/screener/speakeasy');
    expect(screen.queryByRole('link', { name: /allowed senders/i })).not.toBeInTheDocument();
  });

  it('shows live labels in the sidebar as a nested tree and links to label mail views', async () => {
    api.user = adminUser;

    renderRouterAt('/imbox');

    const manageLabels = await screen.findByRole('link', { name: 'Manage labels' });
    expect(manageLabels).toHaveAttribute('href', '/labels');
    expect(await screen.findByText('All labels')).toBeInTheDocument();
    const allLabelsTree = screen.getByRole('list', { name: 'All labels' });
    const sidebarLinks = within(allLabelsTree).getAllByRole('link');
    expect(sidebarLinks.map((link) => link.textContent)).toEqual([
      'Family1',
      'Work3',
      'Work / Receipts',
    ]);

    const sidebarReceiptsLink = within(allLabelsTree).getByRole('link', {
      name: /Work \/ Receipts/,
    });
    expect(sidebarReceiptsLink).toHaveAttribute('href', '/labels/42');
    expect(sidebarReceiptsLink).toHaveAttribute('title', 'Work/Receipts · 0 threads');
    expect(
      fetchMock.mock.calls.some(
        ([url, init]) =>
          String(url) === `${window.location.origin}/api/labels` &&
          (init as RequestInit | undefined)?.credentials === 'include',
      ),
    ).toBe(true);
  });

  it('lets authenticated users reach labels management', async () => {
    api.user = adminUser;

    renderRouterAt('/labels');

    await expectLocation('/labels');
    expect(await screen.findByRole('heading', { name: 'Labels' })).toBeInTheDocument();
    expect((await screen.findAllByText('Work / Receipts')).length).toBeGreaterThan(0);
    expect(
      fetchMock.mock.calls.some(
        ([url, init]) =>
          String(url) === `${window.location.origin}/api/labels` &&
          (init as RequestInit | undefined)?.credentials === 'include',
      ),
    ).toBe(true);
  });

  it('lets authenticated users reach a label mail view', async () => {
    api.user = adminUser;

    renderRouterAt('/labels/42');

    await expectLocation('/labels/42');
    expect(await screen.findByRole('heading', { name: 'Work / Receipts' })).toBeInTheDocument();
    expect(
      fetchMock.mock.calls.some(
        ([url, init]) =>
          String(url) === `${window.location.origin}/api/labels/42/threads` &&
          (init as RequestInit | undefined)?.credentials === 'include',
      ),
    ).toBe(true);
  });

  it('keeps logout available from the sidebar controls', async () => {
    api.user = adminUser;

    renderRouterAt('/imbox');

    expect(await screen.findByRole('button', { name: 'Sign Out' })).toBeEnabled();
  });

  it('updates the auth cache and navigates to imbox after login', async () => {
    renderRouterAt('/login');

    fireEvent.change(await screen.findByLabelText('Email'), {
      target: { value: 'admin@example.com' },
    });
    fireEvent.change(screen.getByLabelText('Password'), {
      target: { value: 'correct horse battery staple' },
    });
    fireEvent.click(screen.getByRole('button', { name: 'Sign in' }));

    await expectLocation('/imbox');
    await waitFor(() => {
      expect(queryClient.getQueryData(queryKeys.me())).toEqual(adminUser);
    });
    expect(api.loginCalls).toEqual([
      { email: 'admin@example.com', password: 'correct horse battery staple' },
    ]);
  });

  it('updates the auth cache and navigates to imbox after first-run setup', async () => {
    api.setup = { wizard_active: true, backend: 'jmap' };

    renderRouterAt('/setup');

    expect(await screen.findByLabelText('Bootstrap token')).toBeRequired();
    expect(screen.getByLabelText('Stalwart admin user')).toBeRequired();
    expect(screen.getByLabelText('Stalwart admin password')).toBeRequired();
    expect(screen.getByLabelText('Stalwart admin user')).toHaveValue('admin');

    fireEvent.change(await screen.findByLabelText('Bootstrap token'), {
      target: { value: 'operator-bootstrap-token' },
    });
    fireEvent.change(screen.getByLabelText('Stalwart admin user'), {
      target: { value: 'root-admin' },
    });
    fireEvent.change(screen.getByLabelText('Stalwart admin password'), {
      target: { value: 'recovery-secret' },
    });
    fireEvent.change(screen.getByLabelText('Admin email'), {
      target: { value: 'admin@example.com' },
    });
    fireEvent.change(screen.getByLabelText('Display name'), {
      target: { value: 'Admin User' },
    });
    fireEvent.change(screen.getByLabelText('Mail domain'), {
      target: { value: 'example.com' },
    });
    fireEvent.change(screen.getByLabelText('Password'), {
      target: { value: 'correct horse battery staple' },
    });
    fireEvent.click(screen.getByRole('button', { name: 'Create admin' }));

    await expectLocation('/imbox');
    await waitFor(() => {
      expect(queryClient.getQueryData(queryKeys.me())).toEqual(adminUser);
    });
    expect(api.setupAdminCalls).toEqual([
      {
        email: 'admin@example.com',
        password: 'correct horse battery staple',
        display_name: 'Admin User',
        domain: 'example.com',
        bootstrap_token: 'operator-bootstrap-token',
        stalwart_admin_username: 'root-admin',
        stalwart_admin_password: 'recovery-secret',
      },
    ]);
  });

  it('renders backend setup error detail in the setup alert', async () => {
    api.setup = { wizard_active: true, backend: 'jmap' };
    api.setupAdminError = {
      status: 400,
      body: {
        error: 'setup_provision_failed',
        detail: 'Unauthorized: invalid Stalwart admin credentials',
      },
    };

    renderRouterAt('/setup');

    fireEvent.change(await screen.findByLabelText('Bootstrap token'), {
      target: { value: 'operator-bootstrap-token' },
    });
    fireEvent.change(screen.getByLabelText('Stalwart admin password'), {
      target: { value: 'wrong-secret' },
    });
    fireEvent.change(screen.getByLabelText('Admin email'), {
      target: { value: 'admin@example.com' },
    });
    fireEvent.change(screen.getByLabelText('Mail domain'), {
      target: { value: 'example.com' },
    });
    fireEvent.change(screen.getByLabelText('Password'), {
      target: { value: 'correct horse battery staple' },
    });
    fireEvent.click(screen.getByRole('button', { name: 'Create admin' }));

    expect(
      await screen.findByText('Unauthorized: invalid Stalwart admin credentials'),
    ).toBeInTheDocument();
  });
});
