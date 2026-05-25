import { useId, useState, type FormEvent } from 'react';
import type { UserView } from '../api/client';
import {
  useAddAdminDomainMutation,
  useAdminDomains,
  useAdminStats,
  useAdminUsers,
  useCreateAdminUserMutation,
  useDeleteAdminDomainMutation,
  useDeleteAdminUserMutation,
  useResetAdminUserPasswordMutation,
} from '../api/query';
import { useAuth } from '../auth/AuthProvider';
import { StateCard } from '../components/StateCard';
import { AppShell } from '../layout/AppShell';
import { adminErrorMessage } from '../lib/errorMessages';

function formatCount(value: number) {
  return new Intl.NumberFormat().format(value);
}

function formatBytes(value: number | null | undefined) {
  if (value == null) {
    return 'Size unavailable';
  }
  if (value < 1024) {
    return `${value} B`;
  }
  const units = ['KB', 'MB', 'GB', 'TB'];
  let amount = value / 1024;
  let unit = units[0];
  for (let i = 1; amount >= 1024 && i < units.length; i += 1) {
    amount /= 1024;
    unit = units[i];
  }
  return `${amount.toFixed(amount >= 10 ? 1 : 2)} ${unit}`;
}

function SystemStatusSection() {
  const stats = useAdminStats();
  const connected = stats.data?.stalwart_status === 'connected';
  const totalEmails = stats.data?.users.reduce((sum, user) => sum + user.total_emails, 0) ?? 0;
  const totalSize = stats.data?.users.reduce<number | null>((sum, user) => {
    if (sum == null || user.total_size_bytes == null) {
      return null;
    }
    return sum + user.total_size_bytes;
  }, 0);

  return (
    <section className="rounded-lg border border-hairline bg-surface p-5">
      <div className="flex items-start justify-between gap-3">
        <div>
          <h2 className="text-lg font-semibold text-ink-primary">System Status</h2>
          <div className="mt-3 flex items-center gap-2 text-sm font-medium text-ink-primary">
            <span
              aria-hidden="true"
              className={`h-2.5 w-2.5 rounded-full ${connected ? 'bg-accent-green' : 'bg-accent-red'}`}
            />
            Stalwart connection: {connected ? 'Connected' : 'Unreachable'}
          </div>
        </div>
        <button
          type="button"
          onClick={() => void stats.refetch()}
          disabled={stats.isFetching}
          className="rounded-full border border-hairline px-3 py-1.5 text-xs font-semibold text-ink-primary transition hover:border-accent-blue hover:text-accent-blue disabled:cursor-not-allowed disabled:opacity-60"
        >
          {stats.isFetching ? 'Refreshing…' : 'Refresh stats'}
        </button>
      </div>
      {stats.isError ? (
        <p role="alert" className="mt-4 text-sm text-accent-red">
          {adminErrorMessage(stats.error, 'Load stats')}
        </p>
      ) : (
        <dl className="mt-5 grid grid-cols-1 gap-3 sm:grid-cols-3">
          <div className="rounded-lg border border-hairline bg-page p-3">
            <dt className="text-xs font-semibold uppercase tracking-wide text-ink-secondary">Total emails</dt>
            <dd className="mt-1 text-2xl font-semibold text-ink-primary">
              {stats.isPending ? '…' : formatCount(totalEmails)}
            </dd>
          </div>
          <div className="rounded-lg border border-hairline bg-page p-3">
            <dt className="text-xs font-semibold uppercase tracking-wide text-ink-secondary">Total users</dt>
            <dd className="mt-1 text-2xl font-semibold text-ink-primary">
              {stats.isPending ? '…' : formatCount(stats.data?.users.length ?? 0)}
            </dd>
          </div>
          <div className="rounded-lg border border-hairline bg-page p-3">
            <dt className="text-xs font-semibold uppercase tracking-wide text-ink-secondary">Storage used</dt>
            <dd className="mt-1 text-2xl font-semibold text-ink-primary">
              {stats.isPending ? '…' : formatBytes(totalSize)}
            </dd>
          </div>
        </dl>
      )}
    </section>
  );
}

function Field({
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
    <label htmlFor={id} className="block text-sm font-medium text-ink-primary">
      {label}
      <input
        id={id}
        type={type}
        value={value}
        onChange={(event) => onChange(event.target.value)}
        autoComplete={autoComplete}
        placeholder={placeholder}
        required={required}
        minLength={minLength}
        className="mt-2 w-full rounded-lg border border-hairline bg-page px-3 py-2 text-ink-primary outline-none ring-accent-blue transition placeholder:text-ink-tertiary focus:border-accent-blue focus:ring-2"
      />
    </label>
  );
}

function FormError({ error, action }: { error: Error | null; action: string }) {
  if (!error) {
    return null;
  }

  return (
    <p role="alert" className="rounded-lg border border-accent-red/30 bg-accent-red/10 px-3 py-2 text-sm text-accent-red">
      {adminErrorMessage(error, action)}
    </p>
  );
}

function CreateUserForm() {
  const emailId = useId();
  const displayNameId = useId();
  const passwordId = useId();
  const [email, setEmail] = useState('');
  const [displayName, setDisplayName] = useState('');
  const [password, setPassword] = useState('');
  const createUser = useCreateAdminUserMutation(undefined, {
    onSuccess: () => {
      setEmail('');
      setDisplayName('');
      setPassword('');
    },
  });

  function onSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    createUser.mutate({
      email,
      password,
      display_name: displayName.trim() || null,
    });
  }

  return (
    <form onSubmit={onSubmit} className="rounded-lg border border-hairline bg-surface p-4">
      <h2 className="text-lg font-semibold text-ink-primary">Create user</h2>
      <div className="mt-4 space-y-3">
        <Field
          id={emailId}
          label="Email"
          type="email"
          value={email}
          onChange={setEmail}
          autoComplete="off"
          placeholder="person@example.org"
        />
        <Field
          id={displayNameId}
          label="Display name"
          value={displayName}
          onChange={setDisplayName}
          autoComplete="off"
          required={false}
          placeholder="Person Name"
        />
        <Field
          id={passwordId}
          label="Initial password"
          type="password"
          value={password}
          onChange={setPassword}
          autoComplete="new-password"
          minLength={12}
        />
        <p className="text-xs text-ink-primary0">Passwords must be at least 12 characters.</p>
        <FormError error={createUser.error} action="Create user" />
        <button
          type="submit"
          disabled={createUser.isPending}
          className="w-full rounded-full bg-accent-blue px-4 py-2 text-sm font-semibold text-white transition hover:bg-accent-blue-hover disabled:cursor-not-allowed disabled:opacity-60"
        >
          {createUser.isPending ? 'Creating…' : 'Create user'}
        </button>
      </div>
    </form>
  );
}

function ResetPasswordForm({ user }: { user: UserView }) {
  const inputId = useId();
  const [password, setPassword] = useState('');
  const resetPassword = useResetAdminUserPasswordMutation(undefined, {
    onSuccess: () => setPassword(''),
  });

  function onSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    resetPassword.mutate({ userId: user.id, password });
  }

  return (
    <form onSubmit={onSubmit} className="mt-3 space-y-2">
      <label htmlFor={inputId} className="block text-xs font-semibold uppercase tracking-wide text-ink-secondary">
        Reset password
      </label>
      <div className="flex gap-2">
        <input
          id={inputId}
          type="password"
          value={password}
          onChange={(event) => setPassword(event.target.value)}
          autoComplete="new-password"
          required
          minLength={12}
          placeholder="New password"
          className="min-w-0 flex-1 rounded-lg border border-hairline bg-page px-3 py-2 text-sm text-ink-primary outline-none ring-accent-blue transition placeholder:text-ink-tertiary focus:border-accent-blue focus:ring-2"
        />
        <button
          type="submit"
          disabled={resetPassword.isPending}
          className="rounded-full border border-hairline px-3 py-2 text-sm font-semibold text-ink-primary transition hover:border-accent-blue hover:text-accent-blue disabled:cursor-not-allowed disabled:opacity-60"
        >
          {resetPassword.isPending ? 'Saving…' : 'Reset'}
        </button>
      </div>
      <FormError error={resetPassword.error} action="Reset password" />
    </form>
  );
}

function UserCard({
  user,
  currentUserId,
  totalEmails,
}: {
  user: UserView;
  currentUserId: number | null;
  totalEmails: number | null;
}) {
  const deleteUser = useDeleteAdminUserMutation();
  const isSelf = currentUserId === user.id;

  return (
    <article className="rounded-lg border border-hairline bg-surface p-4">
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0">
          <h2 className="truncate text-base font-semibold text-ink-primary">{user.email}</h2>
          <p className="mt-1 text-sm text-ink-secondary">{user.display_name || 'No display name'}</p>
          <p className="mt-2 text-xs font-medium text-ink-primary0">
            {totalEmails == null ? 'Email count unavailable' : `${formatCount(totalEmails)} emails`}
          </p>
        </div>
        <div className="flex shrink-0 items-center gap-2">
          {user.is_admin ? (
            <span className="rounded-full border border-accent-blue/40 bg-accent-blue/10 px-2 py-1 text-xs font-semibold text-accent-blue">
              Admin
            </span>
          ) : null}
          <button
            type="button"
            onClick={() => deleteUser.mutate(user.id)}
            disabled={deleteUser.isPending || isSelf}
            title={isSelf ? 'You cannot delete your own admin account.' : undefined}
            className="rounded-full border border-hairline px-3 py-1.5 text-xs font-semibold text-ink-primary transition hover:border-accent-red hover:text-accent-red disabled:cursor-not-allowed disabled:opacity-50"
          >
            {deleteUser.isPending ? 'Deleting…' : 'Delete'}
          </button>
        </div>
      </div>
      {isSelf ? <p className="mt-3 text-xs text-ink-primary0">Signed-in account cannot delete itself.</p> : null}
      <ResetPasswordForm user={user} />
      <FormError error={deleteUser.error} action="Delete user" />
    </article>
  );
}

function UsersSection({
  statsByEmail,
}: {
  statsByEmail: ReadonlyMap<string, number>;
}) {
  const users = useAdminUsers();
  const { user: currentUser } = useAuth();

  if (users.isPending) {
    return <StateCard title="Loading users" body="Reading users from Stalwart management…" />;
  }

  if (users.isError) {
    return <StateCard title="Could not load users" body={adminErrorMessage(users.error, 'Load users')} />;
  }

  return (
    <div className="space-y-3">
      {users.data.users.length === 0 ? (
        <StateCard title="No users" body="Create the first mailbox user with the form below." />
      ) : (
        users.data.users.map((user) => (
          <UserCard
            key={user.id}
            user={user}
            currentUserId={currentUser?.id ?? null}
            totalEmails={statsByEmail.get(user.email.toLowerCase()) ?? null}
          />
        ))
      )}
      <CreateUserForm />
    </div>
  );
}

function AddDomainForm() {
  const domainId = useId();
  const [domain, setDomain] = useState('');
  const addDomain = useAddAdminDomainMutation(undefined, {
    onSuccess: () => setDomain(''),
  });

  function onSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    addDomain.mutate({ domain });
  }

  return (
    <form onSubmit={onSubmit} className="rounded-lg border border-hairline bg-surface p-4">
      <h2 className="text-lg font-semibold text-ink-primary">Add domain</h2>
      <div className="mt-4 space-y-3">
        <Field
          id={domainId}
          label="Domain"
          value={domain}
          onChange={setDomain}
          autoComplete="off"
          placeholder="example.org"
        />
        <FormError error={addDomain.error} action="Add domain" />
        <button
          type="submit"
          disabled={addDomain.isPending}
          className="w-full rounded-full bg-accent-blue px-4 py-2 text-sm font-semibold text-white transition hover:bg-accent-blue-hover disabled:cursor-not-allowed disabled:opacity-60"
        >
          {addDomain.isPending ? 'Adding…' : 'Add domain'}
        </button>
      </div>
    </form>
  );
}

function DomainsSection() {
  const domains = useAdminDomains();
  const deleteDomain = useDeleteAdminDomainMutation();

  return (
    <div className="space-y-4">
      <section className="rounded-lg border border-hairline bg-surface p-5">
        <h2 className="text-lg font-semibold text-ink-primary">Domains</h2>
        {domains.isPending ? (
          <p className="mt-4 text-sm text-ink-secondary">Loading domains…</p>
        ) : domains.isError ? (
          <p role="alert" className="mt-4 text-sm text-accent-red">
            {adminErrorMessage(domains.error, 'Load domains')}
          </p>
        ) : domains.data.domains.length === 0 ? (
          <p className="mt-4 text-sm text-ink-secondary">No domains configured yet.</p>
        ) : (
          <ul className="mt-4 space-y-2">
            {domains.data.domains.map((domain) => (
              <li key={domain} className="flex items-center justify-between gap-3 rounded-lg border border-hairline bg-page px-3 py-2">
                <span className="min-w-0 truncate font-medium text-ink-primary">{domain}</span>
                <button
                  type="button"
                  onClick={() => deleteDomain.mutate(domain)}
                  disabled={deleteDomain.isPending}
                  className="rounded-full border border-hairline px-3 py-1.5 text-xs font-semibold text-ink-primary transition hover:border-accent-red hover:text-accent-red disabled:cursor-not-allowed disabled:opacity-60"
                >
                  Delete
                </button>
              </li>
            ))}
          </ul>
        )}
        <FormError error={deleteDomain.error} action="Delete domain" />
      </section>
      <AddDomainForm />
    </div>
  );
}

function AdminList() {
  const stats = useAdminStats();
  const statsByEmail = new Map(
    stats.data?.users.map((userStats) => [userStats.email.toLowerCase(), userStats.total_emails]) ?? [],
  );

  return (
    <div className="space-y-4">
      <SystemStatusSection />
      <UsersSection statsByEmail={statsByEmail} />
    </div>
  );
}

function ForbiddenAdmin() {
  return (
    <AppShell
      title="Forbidden"
      description="Admin settings are restricted to instance administrators."
      list={
        <StateCard
          title="Admin access required"
          body="Ask an existing admin to grant access if you need to manage users or domains."
        />
      }
      reading={
        <div className="rounded-lg border border-hairline bg-surface p-6">
          <h2 className="text-lg font-semibold text-ink-primary">403 Forbidden</h2>
          <p className="mt-3 text-sm leading-6 text-ink-secondary">
            Your account is signed in, but it is not marked as an administrator.
          </p>
        </div>
      }
    />
  );
}

export function AdminPage() {
  const { user, loading } = useAuth();

  if (loading) {
    return <AppShell title="Admin" description="Loading admin permissions…" list={<StateCard title="Loading" body="Checking your account permissions…" />} />;
  }

  if (!user?.is_admin) {
    return <ForbiddenAdmin />;
  }

  return (
    <AppShell
      title="Admin"
      description="Manage mailbox users and accepted mail domains."
      list={<AdminList />}
      reading={
        <div className="space-y-4">
          <section className="rounded-lg border border-hairline bg-surface p-5">
            <h2 className="text-lg font-semibold text-ink-primary">Operator settings</h2>
            <p className="mt-3 text-sm leading-6 text-ink-secondary">
              User and domain operations call the hail API, which forwards changes to Stalwart management and refreshes these lists after each mutation.
            </p>
          </section>
          <DomainsSection />
        </div>
      }
    />
  );
}
