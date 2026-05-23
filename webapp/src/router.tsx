import { useEffect, useState, type FormEvent } from 'react';
import {
  Link,
  Outlet,
  createRootRoute,
  createRoute,
  createRouter,
  redirect,
  useNavigate,
} from '@tanstack/react-router';
import { HailApiError } from './api/client';
import {
  defaultApiClient,
  useLoginMutation,
  useSetupAdminMutation,
  useSetupState,
} from './api/query';
import { AuthProvider, useAuth } from './auth/AuthProvider';
import { queryClient } from './lib/queryClient';

function AppShell() {
  return (
    <AuthProvider>
      <Outlet />
    </AuthProvider>
  );
}

function CenteredPage({ children }: { children: React.ReactNode }) {
  return (
    <main className="min-h-screen bg-slate-950 px-6 py-12 text-slate-50">
      <section className="mx-auto flex min-h-[calc(100vh-6rem)] w-full max-w-md flex-col justify-center">
        <div className="mb-8 text-center">
          <p className="text-sm font-medium uppercase tracking-[0.35em] text-sky-300">
            hail
          </p>
          <h1 className="mt-3 text-4xl font-semibold tracking-tight">hail</h1>
        </div>
        {children}
      </section>
    </main>
  );
}

function TextInput({
  id,
  label,
  type = 'text',
  value,
  onChange,
  autoComplete,
  required = true,
  minLength,
  placeholder,
}: {
  id: string;
  label: string;
  type?: string;
  value: string;
  onChange: (value: string) => void;
  autoComplete?: string;
  required?: boolean;
  minLength?: number;
  placeholder?: string;
}) {
  return (
    <label className="block text-sm font-medium text-slate-200" htmlFor={id}>
      {label}
      <input
        id={id}
        type={type}
        value={value}
        onChange={(event) => onChange(event.target.value)}
        autoComplete={autoComplete}
        required={required}
        minLength={minLength}
        placeholder={placeholder}
        className="mt-2 w-full rounded-lg border border-slate-700 bg-slate-900 px-3 py-2 text-slate-50 outline-none ring-sky-400 transition focus:border-sky-400 focus:ring-2"
      />
    </label>
  );
}

function ErrorMessage({ message }: { message: string | null }) {
  if (message === null) {
    return null;
  }

  return (
    <p className="rounded-lg border border-red-800 bg-red-950/70 px-3 py-2 text-sm text-red-100">
      {message}
    </p>
  );
}

function apiErrorMessage(error: unknown, fallback: string) {
  if (error instanceof HailApiError) {
    if (error.status === 401) {
      return 'Email or password was not accepted.';
    }
    if (error.status === 409) {
      return 'Setup is no longer active. Try signing in instead.';
    }
    if (error.status === 422 || error.status === 400) {
      return 'Check the form values and try again.';
    }
  }

  return fallback;
}

function LoginPage() {
  const navigate = useNavigate();
  const [email, setEmail] = useState('');
  const [password, setPassword] = useState('');
  const login = useLoginMutation(defaultApiClient, {
    onSuccess: () => {
      void navigate({ to: '/imbox' });
    },
  });

  function onSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    login.mutate({ email, password });
  }

  return (
    <CenteredPage>
      <form
        onSubmit={onSubmit}
        className="rounded-2xl border border-slate-800 bg-slate-900/70 p-6 shadow-2xl shadow-slate-950"
      >
        <h2 className="text-2xl font-semibold">Sign in</h2>
        <p className="mt-2 text-sm text-slate-400">
          Use your hail email and password to continue.
        </p>
        <div className="mt-6 space-y-4">
          <TextInput
            id="email"
            label="Email"
            type="email"
            value={email}
            onChange={setEmail}
            autoComplete="email"
          />
          <TextInput
            id="password"
            label="Password"
            type="password"
            value={password}
            onChange={setPassword}
            autoComplete="current-password"
          />
          <ErrorMessage
            message={
              login.error
                ? apiErrorMessage(login.error, 'Sign in failed. Try again.')
                : null
            }
          />
          <button
            type="submit"
            disabled={login.isPending}
            className="w-full rounded-lg bg-sky-400 px-4 py-2 font-semibold text-slate-950 transition hover:bg-sky-300 disabled:cursor-not-allowed disabled:opacity-60"
          >
            {login.isPending ? 'Signing in…' : 'Sign in'}
          </button>
        </div>
      </form>
    </CenteredPage>
  );
}

function setupInactiveMessage(reason: string | undefined) {
  if (reason === 'config_admin_set') {
    return 'Setup is disabled because an admin is configured in hail.toml.';
  }
  if (reason === 'admin_user_exists') {
    return 'Setup is complete because an admin user already exists.';
  }
  return 'Setup is not active for this instance.';
}

function SetupPage() {
  const navigate = useNavigate();
  const setupState = useSetupState();
  const [email, setEmail] = useState('');
  const [displayName, setDisplayName] = useState('');
  const [domain, setDomain] = useState('');
  const [password, setPassword] = useState('');
  const setupAdmin = useSetupAdminMutation(defaultApiClient, {
    onSuccess: () => {
      void navigate({ to: '/imbox' });
    },
  });

  useEffect(() => {
    const nextDomain = email.split('@')[1] ?? '';
    if (nextDomain.length > 0) {
      setDomain(nextDomain);
    }
  }, [email]);

  function onSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    setupAdmin.mutate({
      email,
      password,
      display_name: displayName.trim() || null,
      domain,
    });
  }

  if (setupState.isPending) {
    return (
      <CenteredPage>
        <p className="text-center text-slate-300">Checking setup state…</p>
      </CenteredPage>
    );
  }

  if (setupState.isError) {
    return (
      <CenteredPage>
        <div className="rounded-2xl border border-red-800 bg-red-950/70 p-6">
          <h2 className="text-xl font-semibold">Setup unavailable</h2>
          <p className="mt-2 text-sm text-red-100">
            Could not read setup state. Refresh and try again.
          </p>
        </div>
      </CenteredPage>
    );
  }

  if (!setupState.data.wizard_active) {
    return (
      <CenteredPage>
        <div className="rounded-2xl border border-slate-800 bg-slate-900/70 p-6">
          <h2 className="text-2xl font-semibold">Setup inactive</h2>
          <p className="mt-2 text-sm text-slate-300">
            {setupInactiveMessage(setupState.data.reason)}
          </p>
          <Link
            to="/login"
            className="mt-6 inline-flex rounded-lg bg-sky-400 px-4 py-2 font-semibold text-slate-950 transition hover:bg-sky-300"
          >
            Go to login
          </Link>
        </div>
      </CenteredPage>
    );
  }

  return (
    <CenteredPage>
      <form
        onSubmit={onSubmit}
        className="rounded-2xl border border-slate-800 bg-slate-900/70 p-6 shadow-2xl shadow-slate-950"
      >
        <h2 className="text-2xl font-semibold">First-run setup</h2>
        <p className="mt-2 text-sm text-slate-400">
          Create the first admin account for this hail instance.
        </p>
        <div className="mt-6 space-y-4">
          <TextInput
            id="setup-email"
            label="Admin email"
            type="email"
            value={email}
            onChange={setEmail}
            autoComplete="email"
            placeholder="you@example.com"
          />
          <TextInput
            id="display-name"
            label="Display name"
            value={displayName}
            onChange={setDisplayName}
            autoComplete="name"
            required={false}
          />
          <TextInput
            id="domain"
            label="Mail domain"
            value={domain}
            onChange={setDomain}
            autoComplete="off"
            placeholder="example.com"
          />
          <TextInput
            id="setup-password"
            label="Password"
            type="password"
            value={password}
            onChange={setPassword}
            autoComplete="new-password"
            minLength={12}
          />
          <p className="text-xs text-slate-500">
            Password must be at least 12 characters. The email must belong to
            the mail domain.
          </p>
          <ErrorMessage
            message={
              setupAdmin.error
                ? apiErrorMessage(
                    setupAdmin.error,
                    'Setup failed. Check the values and try again.',
                  )
                : null
            }
          />
          <button
            type="submit"
            disabled={setupAdmin.isPending}
            className="w-full rounded-lg bg-sky-400 px-4 py-2 font-semibold text-slate-950 transition hover:bg-sky-300 disabled:cursor-not-allowed disabled:opacity-60"
          >
            {setupAdmin.isPending ? 'Creating admin…' : 'Create admin'}
          </button>
        </div>
      </form>
    </CenteredPage>
  );
}

function ImboxPage() {
  const { user, logout, logoutLoading } = useAuth();

  return (
    <main className="min-h-screen bg-slate-950 px-6 py-8 text-slate-50">
      <section className="mx-auto max-w-4xl">
        <header className="flex items-center justify-between gap-4 border-b border-slate-800 pb-6">
          <div>
            <p className="text-sm font-medium uppercase tracking-[0.35em] text-sky-300">
              hail
            </p>
            <h1 className="mt-2 text-4xl font-semibold tracking-tight">Imbox</h1>
          </div>
          <button
            type="button"
            onClick={logout}
            disabled={logoutLoading}
            className="rounded-lg border border-slate-700 px-4 py-2 text-sm font-semibold text-slate-100 transition hover:border-sky-400 hover:text-sky-200 disabled:cursor-not-allowed disabled:opacity-60"
          >
            {logoutLoading ? 'Signing out…' : 'Logout'}
          </button>
        </header>
        <div className="mt-8 rounded-2xl border border-slate-800 bg-slate-900/70 p-6">
          <p className="text-sm text-slate-400">Signed in as</p>
          <p className="mt-1 text-lg font-medium">{user?.email}</p>
          <p className="mt-6 text-slate-300">
            Mail lists arrive later. This placeholder proves the authenticated
            shell is wired up.
          </p>
        </div>
      </section>
    </main>
  );
}

const rootRoute = createRootRoute({
  component: AppShell,
});

const indexRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/',
  beforeLoad: async () => {
    const setup = await queryClient.ensureQueryData({
      queryKey: ['hail', 'setup', 'state'],
      queryFn: () => defaultApiClient.getSetupState(),
    });

    if (setup.wizard_active) {
      throw redirect({ to: '/setup' });
    }

    let authenticated = false;
    try {
      await queryClient.ensureQueryData({
        queryKey: ['hail', 'auth', 'me'],
        queryFn: () => defaultApiClient.me(),
        retry: false,
      });
      authenticated = true;
    } catch {
      authenticated = false;
    }

    throw redirect({ to: authenticated ? '/imbox' : '/login' });
  },
});

const loginRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/login',
  component: LoginPage,
});

const setupRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/setup',
  component: SetupPage,
});

const imboxRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/imbox',
  beforeLoad: async () => {
    try {
      await queryClient.ensureQueryData({
        queryKey: ['hail', 'auth', 'me'],
        queryFn: () => defaultApiClient.me(),
        retry: false,
      });
    } catch {
      throw redirect({ to: '/login' });
    }
  },
  component: ImboxPage,
});

const routeTree = rootRoute.addChildren([
  indexRoute,
  loginRoute,
  setupRoute,
  imboxRoute,
]);

export const router = createRouter({
  routeTree,
});

declare module '@tanstack/react-router' {
  interface Register {
    router: typeof router;
  }
}
