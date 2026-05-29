import { useId, useState, type FormEvent } from 'react';
import type { UserView } from '../api/client';
import {
  useAddAdminDomainMutation,
  useAdminDomains,
  useAdminStats,
  useAdminUsers,
  useCreateInviteMutation,
  useDeleteAdminDomainMutation,
  useDeleteAdminUserMutation,
  useResetAdminUserPasswordMutation,
} from '../api/query';
import { useAuth } from '../auth/AuthProvider';
import { StateCard } from '../components/StateCard';
import { Alert, AlertDescription } from '../components/ui/alert';
import { Badge } from '../components/ui/badge';
import { Button } from '../components/ui/button';
import {
  Card,
  CardAction,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from '../components/ui/card';
import {
  Field as UiField,
  FieldError,
  FieldGroup,
  FieldLabel,
} from '../components/ui/field';
import { Input } from '../components/ui/input';
import { AppShell } from '../layout/AppShell';
import { adminErrorMessage } from '../lib/errorMessages';

function formatCount(value: number) {
  return new Intl.NumberFormat().format(value);
}

function formatBytes(value: number | null | undefined) {
  if (value == null) return 'Size unavailable';
  if (value < 1024) return `${value} B`;
  const units = ['KB', 'MB', 'GB', 'TB'];
  let amount = value / 1024;
  let unit = units[0];
  for (let i = 1; amount >= 1024 && i < units.length; i += 1) {
    amount /= 1024;
    unit = units[i];
  }
  return `${amount.toFixed(amount >= 10 ? 1 : 2)} ${unit}`;
}

function AdminTextField({
  id,
  label,
  value,
  onChange,
  type = 'text',
  autoComplete,
  placeholder,
  required = true,
  minLength,
}: {
  id: string;
  label: string;
  value: string;
  onChange: (value: string) => void;
  type?: string;
  autoComplete?: string;
  placeholder?: string;
  required?: boolean;
  minLength?: number;
}) {
  return (
    <UiField>
      <FieldLabel htmlFor={id}>{label}</FieldLabel>
      <Input
        id={id}
        type={type}
        value={value}
        onChange={(event) => onChange(event.target.value)}
        autoComplete={autoComplete}
        placeholder={placeholder}
        required={required}
        minLength={minLength}
      />
    </UiField>
  );
}

function FormError({ error, action }: { error: Error | null; action: string }) {
  if (!error) return null;
  return <FieldError>{adminErrorMessage(error, action)}</FieldError>;
}

function SystemStatusSection() {
  const stats = useAdminStats();
  const connected = stats.data?.stalwart_status === 'connected';
  const totalEmails = stats.data?.users.reduce((sum, user) => sum + user.total_emails, 0) ?? 0;
  const totalSize = stats.data?.users.reduce<number | null>((sum, user) => {
    if (sum == null || user.total_size_bytes == null) return null;
    return sum + user.total_size_bytes;
  }, 0);

  return (
    <Card>
      <CardHeader>
        <CardTitle role="heading" aria-level={2}>System Status</CardTitle>
        <CardDescription className="flex items-center gap-2">
          Stalwart connection:
          <Badge variant={connected ? 'secondary' : 'destructive'}>
            {connected ? 'Connected' : 'Unreachable'}
          </Badge>
        </CardDescription>
        <CardAction>
          <Button
            type="button"
            variant="outline"
            size="sm"
            onClick={() => void stats.refetch()}
            disabled={stats.isFetching}
          >
            {stats.isFetching ? 'Refreshing…' : 'Refresh stats'}
          </Button>
        </CardAction>
      </CardHeader>
      <CardContent>
        {stats.isError ? (
          <Alert variant="destructive">
            <AlertDescription>{adminErrorMessage(stats.error, 'Load stats')}</AlertDescription>
          </Alert>
        ) : (
          <dl className="grid grid-cols-1 gap-3 sm:grid-cols-3">
            {[
              ['Total emails', stats.isPending ? '…' : formatCount(totalEmails)],
              ['Total users', stats.isPending ? '…' : formatCount(stats.data?.users.length ?? 0)],
              ['Storage used', stats.isPending ? '…' : formatBytes(totalSize)],
            ].map(([label, value]) => (
              <Card key={label} size="sm" className="bg-muted/40 shadow-none">
                <CardContent>
                  <dt className="text-xs font-semibold uppercase tracking-wide text-muted-foreground">{label}</dt>
                  <dd className="mt-1 text-2xl font-semibold">{value}</dd>
                </CardContent>
              </Card>
            ))}
          </dl>
        )}
      </CardContent>
    </Card>
  );
}

function CreateUserForm() {
  const emailId = useId();
  const displayNameId = useId();
  const [email, setEmail] = useState('');
  const [displayName, setDisplayName] = useState('');
  const [inviteUrl, setInviteUrl] = useState('');
  const createInvite = useCreateInviteMutation(undefined, {
    onSuccess: (data) => {
      setEmail('');
      setDisplayName('');
      setInviteUrl(data.invite.invite_url);
    },
  });

  function onSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    createInvite.mutate({ email, display_name: displayName.trim() || null });
  }

  return (
    <Card>
      <form onSubmit={onSubmit}>
        <CardHeader>
          <CardTitle role="heading" aria-level={2}>Invite user</CardTitle>
          <CardDescription className="leading-6">
            Send an expiring invite link so the user sets their own password. The
            link is shown once here; copy it before leaving this page.
          </CardDescription>
        </CardHeader>
        <CardContent>
          <FieldGroup>
            <AdminTextField id={emailId} label="Email" type="email" value={email} onChange={setEmail} autoComplete="off" placeholder="person@example.org" />
            <AdminTextField id={displayNameId} label="Display name" value={displayName} onChange={setDisplayName} autoComplete="off" required={false} placeholder="Person Name" />
            <FormError error={createInvite.error} action="Create invite" />
            {inviteUrl ? (
              <Alert role="status">
                <AlertDescription>
                  <div className="flex flex-col gap-2">
                    <span className="font-medium text-card-foreground">Invite link created</span>
                    <Input readOnly value={inviteUrl} onFocus={(event) => event.currentTarget.select()} aria-label="Invite link" />
                  </div>
                </AlertDescription>
              </Alert>
            ) : null}
            <Button type="submit" disabled={createInvite.isPending} className="w-full">
              {createInvite.isPending ? 'Creating invite…' : 'Create invite link'}
            </Button>
          </FieldGroup>
        </CardContent>
      </form>
    </Card>
  );
}

function ResetPasswordForm({ user }: { user: UserView }) {
  const inputId = useId();
  const [password, setPassword] = useState('');
  const [resetSucceeded, setResetSucceeded] = useState(false);
  const resetPassword = useResetAdminUserPasswordMutation(undefined, {
    onSuccess: () => {
      setPassword('');
      setResetSucceeded(true);
    },
  });

  function onSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    resetPassword.mutate({ userId: user.id, password });
  }

  return (
    <form onSubmit={onSubmit} className="flex flex-col gap-2">
      <FieldGroup>
        <UiField>
          <FieldLabel htmlFor={inputId} className="text-xs font-semibold uppercase tracking-wide text-muted-foreground">Reset password</FieldLabel>
          <div className="flex gap-2">
            <Input id={inputId} type="password" value={password} onChange={(event) => { setPassword(event.target.value); setResetSucceeded(false); }} autoComplete="new-password" required minLength={12} placeholder="New password" className="min-w-0 flex-1" />
            <Button type="submit" variant="outline" disabled={resetPassword.isPending}>{resetPassword.isPending ? 'Saving…' : 'Reset'}</Button>
          </div>
          {resetSucceeded ? <p role="status" className="text-xs text-muted-foreground">Password reset for {user.email}.</p> : null}
          <FormError error={resetPassword.error} action="Reset password" />
        </UiField>
      </FieldGroup>
    </form>
  );
}

function UserCard({ user, currentUserId, totalEmails }: { user: UserView; currentUserId: number | null; totalEmails: number | null }) {
  const deleteUser = useDeleteAdminUserMutation();
  const isSelf = currentUserId === user.id;

  return (
    <Card size="sm">
      <CardHeader>
        <CardTitle role="heading" aria-level={2} className="truncate">{user.email}</CardTitle>
        <CardDescription>{user.display_name || 'No display name'}</CardDescription>
        <CardAction>
          <div className="flex shrink-0 items-center gap-2">
            {user.is_admin ? <Badge variant="secondary">Admin</Badge> : null}
            <Button
              type="button"
              variant="destructive"
              size="sm"
              onClick={() => {
                if (window.confirm(`Delete user ${user.email}? This cannot be undone.`)) {
                  deleteUser.mutate(user.id);
                }
              }}
              disabled={deleteUser.isPending || isSelf}
              title={isSelf ? 'You cannot delete your own admin account.' : undefined}
              aria-label={`Delete user ${user.email}`}
            >
              {deleteUser.isPending ? 'Deleting…' : 'Delete'}
            </Button>
          </div>
        </CardAction>
      </CardHeader>
      <CardContent className="flex flex-col gap-3">
        <p className="text-xs font-medium text-muted-foreground">{totalEmails == null ? 'Email count unavailable' : `${formatCount(totalEmails)} emails`}</p>
        {isSelf ? <p className="text-xs text-muted-foreground">Signed-in account cannot delete itself.</p> : null}
        <ResetPasswordForm user={user} />
        <FormError error={deleteUser.error} action="Delete user" />
      </CardContent>
    </Card>
  );
}

function UsersSection({ statsByEmail }: { statsByEmail: ReadonlyMap<string, number> }) {
  const users = useAdminUsers();
  const { user: currentUser } = useAuth();

  if (users.isPending) return <StateCard title="Loading users" body="Reading users from Stalwart management…" />;
  if (users.isError) return <StateCard title="Could not load users" body={adminErrorMessage(users.error, 'Load users')} />;

  return (
    <div className="flex flex-col gap-3">
      {users.data.users.length === 0 ? (
        <StateCard title="No users" body="Create the first mailbox user with the form below." />
      ) : (
        users.data.users.map((user) => <UserCard key={user.id} user={user} currentUserId={currentUser?.id ?? null} totalEmails={statsByEmail.get(user.email.toLowerCase()) ?? null} />)
      )}
      <CreateUserForm />
    </div>
  );
}

function AddDomainForm() {
  const domainId = useId();
  const [domain, setDomain] = useState('');
  const addDomain = useAddAdminDomainMutation(undefined, { onSuccess: () => setDomain('') });

  function onSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    addDomain.mutate({ domain });
  }

  return (
    <Card>
      <form onSubmit={onSubmit}>
        <CardHeader><CardTitle role="heading" aria-level={2}>Add domain</CardTitle></CardHeader>
        <CardContent>
          <FieldGroup>
            <AdminTextField id={domainId} label="Domain" value={domain} onChange={setDomain} autoComplete="off" placeholder="example.org" />
            <FormError error={addDomain.error} action="Add domain" />
            <Button type="submit" disabled={addDomain.isPending} className="w-full">{addDomain.isPending ? 'Adding…' : 'Add domain'}</Button>
          </FieldGroup>
        </CardContent>
      </form>
    </Card>
  );
}

function DomainsSection() {
  const domains = useAdminDomains();
  const deleteDomain = useDeleteAdminDomainMutation();

  return (
    <div className="flex flex-col gap-4">
      <Card>
        <CardHeader>
          <CardTitle role="heading" aria-level={2}>Domains</CardTitle>
          <CardDescription className="leading-6">Domains are Stalwart-wide: create your shared mail domain once, then create multiple mailbox users under it. Deleting a domain does not restart Stalwart, but will stop Stalwart accepting mail for that domain.</CardDescription>
        </CardHeader>
        <CardContent className="flex flex-col gap-3">
          {domains.isPending ? (
            <p className="text-sm text-muted-foreground">Loading domains…</p>
          ) : domains.isError ? (
            <Alert variant="destructive"><AlertDescription>{adminErrorMessage(domains.error, 'Load domains')}</AlertDescription></Alert>
          ) : domains.data.domains.length === 0 ? (
            <p className="text-sm text-muted-foreground">No domains configured yet.</p>
          ) : (
            <div className="flex flex-col gap-2">
              {domains.data.domains.map((domain) => (
                <Card key={domain} size="sm" className="bg-muted/40 shadow-none">
                  <CardContent className="flex items-center justify-between gap-3">
                    <span className="min-w-0 truncate font-medium">{domain}</span>
                    <Button
                      type="button"
                      variant="destructive"
                      size="sm"
                      onClick={() => {
                        if (window.confirm(`Delete domain ${domain}? Stalwart will stop accepting mail for this domain.`)) {
                          deleteDomain.mutate(domain);
                        }
                      }}
                      disabled={deleteDomain.isPending}
                      aria-label={`Delete domain ${domain}`}
                    >
                      Delete
                    </Button>
                  </CardContent>
                </Card>
              ))}
            </div>
          )}
          <FormError error={deleteDomain.error} action="Delete domain" />
        </CardContent>
      </Card>
      <AddDomainForm />
    </div>
  );
}

function AdminList() {
  const stats = useAdminStats();
  const statsByEmail = new Map(stats.data?.users.map((userStats) => [userStats.email.toLowerCase(), userStats.total_emails]) ?? []);
  return <div className="flex flex-col gap-4"><SystemStatusSection /><UsersSection statsByEmail={statsByEmail} /></div>;
}

function ForbiddenAdmin() {
  return (
    <AppShell
      title="Forbidden"
      description="Admin settings are restricted to instance administrators."
      list={<StateCard title="Admin access required" body="Ask an existing admin to grant access if you need to manage users or domains." />}
      reading={
        <Card>
          <CardHeader>
            <CardTitle role="heading" aria-level={2}>403 Forbidden</CardTitle>
            <CardDescription className="leading-6">Your account is signed in, but it is not marked as an administrator.</CardDescription>
          </CardHeader>
        </Card>
      }
    />
  );
}

export function AdminPage() {
  const { user, loading } = useAuth();

  if (loading) {
    return <AppShell title="Admin" description="Loading admin permissions…" list={<StateCard title="Loading" body="Checking your account permissions…" />} />;
  }
  if (!user?.is_admin) return <ForbiddenAdmin />;

  return (
    <AppShell
      title="Admin"
      description="Manage mailbox users and accepted mail domains."
      list={<AdminList />}
      reading={
        <div className="flex flex-col gap-4">
          <Card>
            <CardHeader>
              <CardTitle role="heading" aria-level
={2}>Operator settings</CardTitle>
              <CardDescription className="leading-6">
                User and domain operations call the hail API, which forwards changes to Stalwart management over HTTP. Creating a user also ensures that user&apos;s email domain exists first, avoiding a risky Stalwart restart or manual config edit for normal shared-domain provisioning.
              </CardDescription>
            </CardHeader>
          </Card>
          <DomainsSection />
        </div>
      }
    />
  );
}
