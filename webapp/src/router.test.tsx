import { QueryClientProvider } from '@tanstack/react-query';
import { RouterProvider } from '@tanstack/react-router';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type {
  LoginRequest,
  MailViewResponse,
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
}

let api: ApiState;
let fetchMock: ReturnType<typeof vi.fn>;

beforeEach(() => {
  api = {
    setup: { wizard_active: false, reason: 'admin_user_exists' },
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
      api.user = adminUser;
      api.setup = { wizard_active: false, reason: 'admin_user_exists' };
      return jsonResponse(adminUser, 201);
    }

    if (url.pathname === '/api/views/imbox' && method === 'GET') {
      return jsonResponse(imbox);
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
    api.setup = { wizard_active: true };

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
    expect(screen.getAllByText('admin@example.com').length).toBeGreaterThan(0);
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
    api.setup = { wizard_active: true };

    renderRouterAt('/setup');

    fireEvent.change(await screen.findByLabelText('Bootstrap token'), {
      target: { value: 'operator-bootstrap-token' },
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
      },
    ]);
  });
});
