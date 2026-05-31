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
import { Alert, AlertDescription } from './components/ui/alert';
import { Button } from './components/ui/button';
import {
  Card,
  CardContent,
  CardDescription,
  CardFooter,
  CardHeader,
  CardTitle,
} from './components/ui/card';
import {
  Field,
  FieldDescription,
  FieldGroup,
  FieldLabel,
} from './components/ui/field';
import { Input } from './components/ui/input';
import { Spinner } from './components/ui/spinner';
import { useTheme } from './hooks/useTheme';
import { formErrorMessage } from './lib/errorMessages';
import { queryClient } from './lib/queryClient';
import { AdminPage } from './routes/AdminPage';
import { AllFilesPage } from './routes/AllFilesPage';
import { ArchivePage } from './routes/ArchivePage';
import { BubbleUpPage } from './routes/BubbleUpPage';
import { ComposerPage } from './routes/ComposerPage';
import { DraftsPage } from './routes/DraftsPage';
import { LabelViewPage } from './routes/LabelViewPage';
import { LabelsManagementPage } from './routes/LabelsManagementPage';
import { MailViewPage } from './routes/MailViewPage';
import { PileSectionPage } from './routes/PileSectionPage';
import { ScreenerPage } from './routes/ScreenerPage';
import { ScreenerSpeakeasyPage } from './routes/ScreenerSpeakeasyPage';
import { SearchPage } from './routes/SearchPage';
import { ScheduledSendsPage } from './routes/ScheduledSendsPage';
import { ProviderAccountsPage } from './routes/ProviderAccountsPage';
import { PreferencesPage } from './routes/PreferencesPage';
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
    <main className="min-h-screen bg-background px-6 py-12 text-foreground">
      <section className="mx-auto flex min-h-[calc(100vh-6rem)] w-full max-w-md flex-col justify-center">
        <div className="mb-8 flex flex-col items-center">
          <img src="/logo-icon-transparent.png" alt="hail" className="h-16" />
          <h1 className="mt-3 text-4xl font-semibold tracking-tight text-foreground">
            hail
          </h1>
        </div>
        {children}
      </section>
    </main>
  );
}

function ErrorMessage({ message }: { message: string | null }) {
  if (message === null) {
    return null;
  }

  return (
    <Alert variant="destructive">
      <AlertDescription>{message}</AlertDescription>
    </Alert>
  );
}

function SubmitButton({
  form,
  isPending,
  pendingText,
  children,
}: {
  form?: string;
  isPending: boolean;
  pendingText: string;
  children: ReactNode;
}) {
  return (
    <Button type="submit" form={form} disabled={isPending} className="w-full">
      {isPending ? (
        <>
          <Spinner
            data-icon="inline-start"
            aria-hidden="true"
            role="presentation"
          />
          {pendingText}
        </>
      ) : (
        children
      )}
    </Button>
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
      <Card size="sm">
        <CardHeader>
          <CardTitle role="heading" aria-level={2}>
            Sign in
          </CardTitle>
          <CardDescription>
            Use your hail email and password to continue.
          </CardDescription>
        </CardHeader>
        <CardContent>
          <form id="login-form" onSubmit={onSubmit}>
            <FieldGroup>
              <Field>
                <FieldLabel htmlFor="email">Email</FieldLabel>
                <Input
                  id="email"
                  type="email"
                  value={email}
                  onChange={(event) => setEmail(event.target.value)}
                  autoComplete="email"
                  required
                />
              </Field>
              <Field>
                <FieldLabel htmlFor="password">Password</FieldLabel>
                <Input
                  id="password"
                  type="password"
                  value={password}
                  onChange={(event) => setPassword(event.target.value)}
                  autoComplete="current-password"
                  required
                />
              </Field>
              <ErrorMessage
                message={
                  login.error
                    ? formErrorMessage(
                        login.error,
                        'Sign in failed. Try again.',
                      )
                    : null
                }
              />
            </FieldGroup>
          </form>
        </CardContent>
        <CardFooter>
          <SubmitButton
            form="login-form"
            isPending={login.isPending}
            pendingText="Signing in…"
          >
            Sign in
          </SubmitButton>
        </CardFooter>
      </Card>
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
      <Card size="sm">
        <CardHeader>
          <CardTitle role="heading" aria-level={2}>
            Accept invite
          </CardTitle>
          {!invite.isPending && !invite.isError ? (
            <CardDescription>
              Create a password for {invite.data.email}.
            </CardDescription>
          ) : null}
        </CardHeader>
        <CardContent>
          {invite.isPending ? (
            <p className="text-sm text-muted-foreground">Checking invite…</p>
          ) : invite.isError ? (
            <Alert variant="destructive">
              <AlertDescription>
                This invite is invalid, expired, or already used.
              </AlertDescription>
            </Alert>
          ) : (
            <form id="invite-form" onSubmit={onSubmit}>
              <FieldGroup>
                <Field>
                  <FieldLabel htmlFor="invite-password">Password</FieldLabel>
                  <Input
                    id="invite-password"
                    type="password"
                    value={password}
                    onChange={(event) => setPassword(event.target.value)}
                    autoComplete="new-password"
                    required
                    minLength={12}
                  />
                  <FieldDescription>
                    Password must be at least 12 characters.
                  </FieldDescription>
                </Field>
                <ErrorMessage
                  message={
                    acceptInvite.error
                      ? formErrorMessage(
                          acceptInvite.error,
                          'Invite failed. Try again.',
                        )
                      : null
                  }
                />
              </FieldGroup>
            </form>
          )}
        </CardContent>
        {!invite.isPending && !invite.isError ? (
          <CardFooter>
            <SubmitButton
              form="invite-form"
              isPending={acceptInvite.isPending}
              pendingText="Creating account…"
            >
              Create account
            </SubmitButton>
          </CardFooter>
        ) : null}
      </Card>
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
  const [stalwartAdminUsername, setStalwartAdminUsername] = useState('admin');
  const [stalwartAdminPassword, setStalwartAdminPassword] = useState('');
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
      stalwart_admin_username: stalwartAdminUsername,
      stalwart_admin_password: stalwartAdminPassword,
    });
  }

  if (setupState.isPending) {
    return (
      <CenteredPage>
        <p className="text-center text-muted-foreground">
          Checking setup state…
        </p>
      </CenteredPage>
    );
  }

  if (setupState.isError) {
    return (
      <CenteredPage>
        <Card size="sm">
          <CardHeader>
            <CardTitle role="heading" aria-level={2}>
              Setup unavailable
            </CardTitle>
          </CardHeader>
          <CardContent>
            <Alert variant="destructive">
              <AlertDescription>
                Could not read setup state. Refresh and try again.
              </AlertDescription>
            </Alert>
          </CardContent>
        </Card>
      </CenteredPage>
    );
  }

  if (!setupState.data.wizard_active) {
    return (
      <CenteredPage>
        <Card size="sm">
          <CardHeader>
            <CardTitle role="heading" aria-level={2}>
              Setup inactive
            </CardTitle>
            <CardDescription>
              {setupInactiveMessage(setupState.data.reason)}
            </CardDescription>
          </CardHeader>
          <CardFooter>
            <Button asChild>
              <Link to="/login">Go to login</Link>
            </Button>
          </CardFooter>
        </Card>
      </CenteredPage>
    );
  }

  return (
    <CenteredPage>
      <Card size="sm">
        <CardHeader>
          <CardTitle role="heading" aria-level={2}>
            First-run setup
          </CardTitle>
          <CardDescription>
            Create the first admin mailbox for this hail instance. If Stalwart
            management is configured, hail authenticates to Stalwart
            v0.16&apos;s management REST API with the Stalwart admin credentials
            below, creates the domain and mailbox with a short-lived bearer
            token, and then verifies the mailbox with JMAP. If management is
            disabled, the domain and account must already exist in Stalwart and
            the wizard verifies them with a JMAP login. You need the operator
            bootstrap token from the server environment/config.
          </CardDescription>
        </CardHeader>
        <CardContent>
          <form id="setup-form" onSubmit={onSubmit}>
            <FieldGroup>
              <Field>
                <FieldLabel htmlFor="setup-bootstrap-token">
                  Bootstrap token
                </FieldLabel>
                <Input
                  id="setup-bootstrap-token"
                  type="password"
                  value={bootstrapToken}
                  onChange={(event) => setBootstrapToken(event.target.value)}
                  autoComplete="off"
                  placeholder="Paste HAIL_SETUP__BOOTSTRAP_TOKEN"
                  required
                />
              </Field>
              <Field>
                <FieldLabel htmlFor="stalwart-admin-username">
                  Stalwart admin user
                </FieldLabel>
                <Input
                  id="stalwart-admin-username"
                  value={stalwartAdminUsername}
                  onChange={(event) =>
                    setStalwartAdminUsername(event.target.value)
                  }
                  autoComplete="off"
                  required
                />
              </Field>
              <Field>
                <FieldLabel htmlFor="stalwart-admin-password">
                  Stalwart admin password
                </FieldLabel>
                <Input
                  id="stalwart-admin-password"
                  type="password"
                  value={stalwartAdminPassword}
                  onChange={(event) =>
                    setStalwartAdminPassword(event.target.value)
                  }
                  autoComplete="off"
                  required
                />
                <FieldDescription>
                  Local compose defaults to admin/admin1234 via
                  STALWART_RECOVERY_ADMIN. Use the recovery admin credentials
                  for your Stalwart instance.
                </FieldDescription>
              </Field>
              <Field>
                <FieldLabel htmlFor="setup-email">Admin email</FieldLabel>
                <Input
                  id="setup-email"
                  type="email"
                  value={email}
                  onChange={(event) => setEmail(event.target.value)}
                  autoComplete="email"
                  placeholder="you@example.com"
                  required
                />
              </Field>
              <Field>
                <FieldLabel htmlFor="display-name">Display name</FieldLabel>
                <Input
                  id="display-name"
                  value={displayName}
                  onChange={(event) => setDisplayName(event.target.value)}
                  autoComplete="name"
                />
              </Field>
              <Field>
                <FieldLabel htmlFor="domain">Mail domain</FieldLabel>
                <Input
                  id="domain"
                  value={domain}
                  onChange={(event) => setDomain(event.target.value)}
                  autoComplete="off"
                  placeholder="example.com"
                  required
                />
              </Field>
              <Field>
                <FieldLabel htmlFor="setup-password">Password</FieldLabel>
                <Input
                  id="setup-password"
                  type="password"
                  value={password}
                  onChange={(event) => setPassword(event.target.value)}
                  autoComplete="new-password"
                  required
                  minLength={12}
                />
                <FieldDescription>
                  Password must be at least 12 characters. The email must belong
                  to the mail domain. The wizard accepts domains with or without
                  a trailing dot; the API normalizes to lowercase before
                  provisioning.
                </FieldDescription>
              </Field>
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
            </FieldGroup>
          </form>
        </CardContent>
        <CardFooter>
          <SubmitButton
            form="setup-form"
            isPending={setupAdmin.isPending}
            pendingText="Creating admin…"
          >
            Create admin
          </SubmitButton>
        </CardFooter>
      </Card>
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
  const { replyTo, replyAll, draftId, forward, in_reply_to } =
    composeRoute.useSearch();
  return (
    <ComposerPage
      replyToThreadId={replyTo}
      replyAll={replyAll}
      forwardThreadId={forward}
      inReplyToEmailId={in_reply_to}
      draftId={draftId}
    />
  );
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

const providerAccountsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/provider-accounts',
  beforeLoad: requireAuth,
  component: ProviderAccountsPage,
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

const labelsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/labels',
  beforeLoad: requireAuth,
  component: LabelsManagementPage,
});

const labelRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/labels/$labelId',
  beforeLoad: requireAuth,
  component: () => {
    const { labelId } = labelRoute.useParams();
    return <LabelViewPage labelId={Number(labelId)} />;
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
  in_reply_to?: string;
}

const composeRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/compose',
  beforeLoad: requireAuth,
  validateSearch: (search: Record<string, unknown>): ComposeSearch => ({
    replyTo:
      typeof search.replyTo === 'string' && search.replyTo.length > 0
        ? search.replyTo
        : undefined,
    replyAll:
      search.replyAll === '1' ||
      search.replyAll === 'true' ||
      search.replyAll === true,
    draftId:
      typeof search.draftId === 'string' && search.draftId.length > 0
        ? search.draftId
        : typeof search.draft === 'string' && search.draft.length > 0
          ? search.draft
          : undefined,
    draft:
      typeof search.draft === 'string' && search.draft.length > 0
        ? search.draft
        : undefined,
    forward:
      typeof search.forward === 'string' && search.forward.length > 0
        ? search.forward
        : undefined,
    in_reply_to:
      typeof search.in_reply_to === 'string' && search.in_reply_to.length > 0
        ? search.in_reply_to
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

const preferencesRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/preferences',
  beforeLoad: requireAuth,
  component: PreferencesPage,
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
  providerAccountsRoute,
  allFilesRoute,
  screenerRoute,
  screenedOutRoute,
  screenerSpeakeasyRoute,
  setAsideRoute,
  replyLaterRoute,
  bubbleUpRoute,
  threadRoute,
  threadReplyRoute,
  labelsRoute,
  labelRoute,
  searchRoute,
  trashRoute,
  spamRoute,
  archiveRoute,
  composeRoute,
  adminRoute,
  preferencesRoute,
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
