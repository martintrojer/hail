import { useEffect, useState, type FormEvent, type ReactNode } from 'react';
import {
  Link,
  Outlet,
  createRootRoute,
  createRoute,
  createRouter,
  redirect,
  useNavigate,
} from '@tanstack/react-router';
import { useApiClient } from './api/ApiClientProvider';
import {
  defaultApiClient,
  useAcceptInviteMutation,
  useInvite,
  useLoginMutation,
  useSetupAdminMutation,
  useSetupState,
} from './api/query';
import { AuthProvider } from './auth/AuthProvider';
import { UndoToastProvider } from './components/UndoToastProvider';
import { useTheme } from './hooks/useTheme';
import { formErrorMessage } from './lib/errorMessages';
import { queryClient } from './lib/queryClient';
import { AdminPage } from './routes/AdminPage';
import { AllFilesPage } from './routes/AllFilesPage';
import { ArchivePage } from './routes/ArchivePage';
import { BubbleUpPage } from './routes/BubbleUpPage';
import { ComposerPage } from './routes/ComposerPage';
import { DraftsPage } from './routes/DraftsPage';
import { MailViewPage } from './routes/MailViewPage';
import { PileSectionPage } from './routes/PileSectionPage';
import { ScreenerPage } from './routes/ScreenerPage';
import { ScreenerSpeakeasyPage } from './routes/ScreenerSpeakeasyPage';
import { SearchPage } from './routes/SearchPage';
import { ScheduledSendsPage } from './routes/ScheduledSendsPage';
import { ThreadPage } from './routes/ThreadPage';
import { TrashPage } from './routes/TrashPage';
import { SpamPage } from './routes/SpamPage';
import { ScreenedOutPage } from './routes/ScreenedOutPage';
import { WorkflowsPage } from './routes/WorkflowsPage';

function AppShell() {
  return (
    <AuthProvider>
      <UndoToastProvider>
        <Outlet />
      </UndoToastProvider>
    </AuthProvider>
  );
}

function CenteredPage({ children }: { children: ReactNode }) {
  useTheme();

  return (
    <main className="min-h-screen bg-bg-page px-6 py-12 text-ink-primary">
      <section className="mx-auto flex min-h-[calc(100vh-6rem)] w-full max-w-md flex-col justify-center">
        <div className="mb-8 flex flex-col items-center">
          <img src="/logo-icon-transparent.png" alt="hail" className="h-16" />
          <h1 className="mt-3 text-4xl font-semibold tracking-tight text-ink-primary">hail</h1>
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
    <label className="block text-sm font-medium text-ink-secondary" htmlFor={id}>
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
        className="mt-2 w-full rounded-lg border border-border-menu bg-bg-surface px-3 py-2 text-ink-primary outline-none ring-accent-blue transition placeholder:text-ink-tertiary focus:border-accent-blue focus:ring-2"
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

function LoginPage() {
  const navigate = useNavigate();
  const apiClient = useApiClient();
  const [email, setEmail] = useState('');
  const [password, setPassword] = useState('');
  const login = useLoginMutation(apiClient, {
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
        className="rounded-2xl border border-border-menu bg-bg-surface p-6 shadow-2xl shadow-ink-primary/10"
      >
        <h2 className="text-2xl font-semibold">Sign in</h2>
        <p className="mt-2 text-sm text-ink-secondary">
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
                ? formErrorMessage(login.error, 'Sign in failed. Try again.')
                : null
            }
          />
          <button
            type="submit"
            disabled={login.isPending}
            className="w-full rounded-lg bg-accent-blue px-4 py-2 font-semibold text-white transition hover:bg-accent-blue-hover disabled:cursor-not-allowed disabled:opacity-60"
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

function InvitePage() {
  const navigate = useNavigate();
  const { token } = inviteRoute.useParams();
  const [password, setPassword] = useState('');
  const invite = useInvite(token);
  const acceptInvite = useAcceptInviteMutation(defaultApiClient, {
    onSuccess: () => {
      void navigate({ to: '/imbox' });
    },
  });

  function onSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    acceptInvite.mutate({ token, body: { password } });
  }

  return (
    <CenteredPage>
      <form
        onSubmit={onSubmit}
        className="rounded-2xl border border-border-menu bg-bg-surface p-6 shadow-2xl shadow-ink-primary/10"
      >
        <h2 className="text-2xl font-semibold">Accept invite</h2>
        {invite.isPending ? (
          <p className="mt-4 text-sm text-ink-secondary">Checking invite…</p>
        ) : invite.isError ? (
          <p role="alert" className="mt-4 rounded-lg border border-red-800 bg-red-950/70 px-3 py-2 text-sm text-red-100">
            This invite is invalid, expired, or already used.
          </p>
        ) : (
          <>
            <p className="mt-2 text-sm text-ink-secondary">
              Create a password for {invite.data.email}.
            </p>
            <div className="mt-6 space-y-4">
              <TextInput
                id="invite-password"
                label="Password"
                type="password"
                value={password}
                onChange={setPassword}
                autoComplete="new-password"
                minLength={12}
              />
              <p className="text-xs text-ink-tertiary">Password must be at least 12 characters.</p>
              <ErrorMessage
                message={
                  acceptInvite.error
                    ? formErrorMessage(acceptInvite.error, 'Invite failed. Try again.')
                    : null
                }
              />
              <button
                type="submit"
                disabled={acceptInvite.isPending}
                className="w-full rounded-lg bg-accent-blue px-4 py-2 font-semibold text-white transition hover:bg-accent-blue-hover disabled:cursor-not-allowed disabled:opacity-60"
              >
                {acceptInvite.isPending ? 'Creating account…' : 'Create account'}
              </button>
            </div>
          </>
        )}
      </form>
    </CenteredPage>
  );
}

function SetupPage() {
  const navigate = useNavigate();
  const apiClient = useApiClient();
  const setupState = useSetupState(apiClient);
  const [email, setEmail] = useState('');
  const [displayName, setDisplayName] = useState('');
  const [domain, setDomain] = useState('');
  const [password, setPassword] = useState('');
  const [bootstrapToken, setBootstrapToken] = useState('');
  const setupAdmin = useSetupAdminMutation(apiClient, {
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
      bootstrap_token: bootstrapToken,
    });
  }

  if (setupState.isPending) {
    return (
      <CenteredPage>
        <p className="text-center text-ink-secondary">Checking setup state…</p>
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
        <div className="rounded-2xl border border-border-menu bg-bg-surface p-6">
          <h2 className="text-2xl font-semibold">Setup inactive</h2>
          <p className="mt-2 text-sm text-ink-secondary">
            {setupInactiveMessage(setupState.data.reason)}
          </p>
          <Link
            to="/login"
            className="mt-6 inline-flex rounded-lg bg-accent-blue px-4 py-2 font-semibold text-white transition hover:bg-accent-blue-hover"
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
        className="rounded-2xl border border-border-menu bg-bg-surface p-6 shadow-2xl shadow-ink-primary/10"
      >
        <h2 className="text-2xl font-semibold">First-run setup</h2>
        <p className="mt-2 text-sm text-ink-secondary">
          Create the first admin mailbox for this hail instance. If Stalwart
          management is configured, hail first provisions the mail domain in
          Stalwart and then creates the admin principal; otherwise the domain
          and account must already exist in Stalwart and the wizard verifies
          them with a JMAP login. You need the operator bootstrap token from
          the server environment/config.
        </p>
        <div className="mt-6 space-y-4">
          <TextInput
            id="setup-bootstrap-token"
            label="Bootstrap token"
            type="password"
            value={bootstrapToken}
            onChange={setBootstrapToken}
            autoComplete="off"
            placeholder="Paste HAIL_SETUP__BOOTSTRAP_TOKEN"
          />
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
          <p className="text-xs text-ink-tertiary">
            Password must be at least 12 characters. The email must belong to
            the mail domain. The wizard accepts domains with or without a
            trailing dot; the API normalizes to lowercase before provisioning.
          </p>
          <ErrorMessage
            message={
              setupAdmin.error
                ? formErrorMessage(
                    setupAdmin.error,
                    'Setup failed. Check the values and try again.',
                  )
                : null
            }
          />
          <button
            type="submit"
            disabled={setupAdmin.isPending}
            className="w-full rounded-lg bg-accent-blue px-4 py-2 font-semibold text-white transition hover:bg-accent-blue-hover disabled:cursor-not-allowed disabled:opacity-60"
          >
            {setupAdmin.isPending ? 'Creating admin…' : 'Create admin'}
          </button>
        </div>
      </form>
    </CenteredPage>
  );
}

function ImboxPage() {
  return (
    <MailViewPage
      view="imbox"
      title="Imbox"
      description="Important mail from approved people lands here."
    />
  );
}

function FeedPage() {
  return (
    <MailViewPage
      view="feed"
      title="Feed"
      description="Newsletters and recurring reading can collect here."
    />
  );
}

function PaperTrailPage() {
  return (
    <MailViewPage
      view="papertrail"
      title="Paper Trail"
      description="Receipts, statements, and reference mail will land here."
    />
  );
}

function DraftsRoutePage() {
  return <DraftsPage />;
}

function ScheduledRoutePage() {
  return <ScheduledSendsPage />;
}

function ComposePage() {
  const { replyTo, replyAll, draftId } = composeRoute.useSearch();
  return <ComposerPage replyToThreadId={replyTo} replyAll={replyAll} draftId={draftId} />;
}

function ThreadReplyPage({ threadId }: { threadId: string }) {
  return <ComposerPage replyToThreadId={threadId} />;
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

const inviteRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/invite/$token',
  component: InvitePage,
});

async function requireAuth() {
  try {
    return await queryClient.ensureQueryData({
      queryKey: ['hail', 'auth', 'me'],
      queryFn: () => defaultApiClient.me(),
      retry: false,
    });
  } catch {
    throw redirect({ to: '/login' });
  }
}

const imboxRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/imbox',
  beforeLoad: requireAuth,
  component: ImboxPage,
});

const feedRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/feed',
  beforeLoad: requireAuth,
  component: FeedPage,
});

const paperTrailRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/papertrail',
  beforeLoad: requireAuth,
  component: PaperTrailPage,
});

const draftsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/drafts',
  beforeLoad: requireAuth,
  component: DraftsRoutePage,
});

const scheduledRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/scheduled',
  beforeLoad: requireAuth,
  component: ScheduledRoutePage,
});

const allFilesRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/files',
  beforeLoad: requireAuth,
  component: AllFilesPage,
});

const screenerRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/screener',
  beforeLoad: requireAuth,
  component: ScreenerPage,
});

const screenedOutRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/screened-out',
  beforeLoad: requireAuth,
  component: ScreenedOutPage,
});

const screenerSpeakeasyRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/screener/speakeasy',
  beforeLoad: requireAuth,
  component: ScreenerSpeakeasyPage,
});

const setAsideRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/set-aside',
  beforeLoad: requireAuth,
  component: () => <PileSectionPage kind="set-aside" />,
});

const replyLaterRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/reply-later',
  beforeLoad: requireAuth,
  component: () => <PileSectionPage kind="reply-later" />,
});

const bubbleUpRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/bubble-up',
  beforeLoad: requireAuth,
  component: BubbleUpPage,
});

const threadRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/thread/$threadId',
  beforeLoad: requireAuth,
  validateSearch: (search: Record<string, unknown>) => ({
    from: (search.from as string) || undefined,
  }),
  component: () => {
    const { threadId } = threadRoute.useParams();
    const { from } = threadRoute.useSearch();
    return <ThreadPage threadId={threadId} sourceView={from} />;
  },
});

const threadReplyRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/thread/$threadId/reply',
  beforeLoad: requireAuth,
  component: () => {
    const { threadId } = threadReplyRoute.useParams();
    return <ThreadReplyPage threadId={threadId} />;
  },
});

const searchRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/search',
  beforeLoad: requireAuth,
  component: SearchPage,
});

const trashRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/trash',
  beforeLoad: requireAuth,
  component: TrashPage,
});

const spamRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/spam',
  beforeLoad: requireAuth,
  component: SpamPage,
});

const archiveRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/archive',
  beforeLoad: requireAuth,
  component: ArchivePage,
});

interface ComposeSearch {
  replyTo?: string;
  replyAll?: boolean;
  draftId?: string;
  draft?: string;
  forward?: string;
}

const composeRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/compose',
  beforeLoad: requireAuth,
  validateSearch: (search: Record<string, unknown>): ComposeSearch => ({
    replyTo: typeof search.replyTo === 'string' && search.replyTo.length > 0
      ? search.replyTo
      : undefined,
    replyAll: search.replyAll === '1' || search.replyAll === 'true' || search.replyAll === true,
    draftId: typeof search.draftId === 'string' && search.draftId.length > 0
      ? search.draftId
      : typeof search.draft === 'string' && search.draft.length > 0
        ? search.draft
        : undefined,
    draft: typeof search.draft === 'string' && search.draft.length > 0
      ? search.draft
      : undefined,
    forward: typeof search.forward === 'string' && search.forward.length > 0
      ? search.forward
      : undefined,
  }),
  component: ComposePage,
});

const adminRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/admin',
  beforeLoad: requireAuth,
  component: AdminPage,
});

const workflowsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/workflows',
  beforeLoad: requireAuth,
  component: WorkflowsPage,
});

const routeTree = rootRoute.addChildren([
  indexRoute,
  loginRoute,
  setupRoute,
  inviteRoute,
  imboxRoute,
  feedRoute,
  paperTrailRoute,
  draftsRoute,
  scheduledRoute,
  allFilesRoute,
  screenerRoute,
  screenedOutRoute,
  screenerSpeakeasyRoute,
  setAsideRoute,
  replyLaterRoute,
  bubbleUpRoute,
  threadRoute,
  threadReplyRoute,
  searchRoute,
  trashRoute,
  spamRoute,
  archiveRoute,
  composeRoute,
  adminRoute,
  workflowsRoute,
]);

export const router = createRouter({
  routeTree,
});

declare module '@tanstack/react-router' {
  interface Register {
    router: typeof router;
  }
}
