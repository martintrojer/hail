import { useId, useState, type FormEvent } from 'react';
import { HailApiError, type UserView } from '../api/client';
import {
  useAddAdminDomainMutation,
  useAdminDomains,
  useAdminUsers,
  useCreateAdminUserMutation,
  useDeleteAdminDomainMutation,
  useDeleteAdminUserMutation,
  useResetAdminUserPasswordMutation,
} from '../api/query';
import { useAuth } from '../auth/AuthProvider';
import { AppShell } from '../layout/AppShell';

function adminErrorMessage(error: Error, action: string) {
  if (error instanceof HailApiError) {
    if (error.status === 400 || error.status === 422) {
      return `Check the ${action} values and try again.`;
    }
    if (error.status === 401) {
      return 'Your session expired. Sign in again.';
    }
    if (error.status === 403) {
      return 'Admin access is required.';
    }
    if (error.status === 404) {
      return 'That item no longer exists. Refresh and try again.';
    }
    if (error.status === 501) {
      return 'Stalwart management is not configured for this instance.';
    }
    if (error.status === 502) {
      return 'Stalwart management failed. Try again or check the server logs.';
    }
    return `${action} failed with HTTP ${error.status}.`;
  }

  return `${action} failed. Try again.`;
}

function StateCard({ title, body }: { title: string; body: string }) {
  return (
    <div className="rounded-2xl border border-dashed border-slate-800 bg-slate-900/40 p-8 text-center">
      <p className="text-base font-semibold text-slate-200">{title}</p>
      <p className="mt-2 text-sm text-slate-400">{body}</p>
    </div>
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
    <label htmlFor={id} className="block text-sm font-medium text-slate-200">
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
        className="mt-2 w-full rounded-lg border border-slate-700 bg-slate-950 px-3 py-2 text-slate-100 outline-none ring-sky-400 transition placeholder:text-slate-600 focus:border-sky-400 focus:ring-2"
      />
    </label>
  );
}

function FormError({ error, action }: { error: Error | null; action: string }) {
  if (!error) {
    return null;
  }

  return (
    <p role="alert" className="rounded-lg border border-red-800 bg-red-950/70 px-3 py-2 text-sm text-red-100">
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
    <form onSubmit={onSubmit} className="rounded-2xl border border-slate-800 bg-slate-900/60 p-4">
      <h2 className="text-lg font-semibold text-slate-100">Create user</h2>
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
        <p className="text-xs text-slate-500">Passwords must be at least 12 characters.</p>
        <FormError error={createUser.error} action="Create user" />
        <button
          type="submit"
          disabled={createUser.isPending}
          className="w-full rounded-lg bg-sky-400 px-4 py-2 text-sm font-semibold text-slate-950 transition hover:bg-sky-300 disabled:cursor-not-allowed disabled:opacity-60"
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
      <label htmlFor={inputId} className="block text-xs font-semibold uppercase tracking-wide text-slate-400">
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
          className="min-w-0 flex-1 rounded-lg border border-slate-700 bg-slate-950 px-3 py-2 text-sm text-slate-100 outline-none ring-sky-400 transition placeholder:text-slate-600 focus:border-sky-400 focus:ring-2"
        />
        <button
          type="submit"
          disabled={resetPassword.isPending}
          className="rounded-lg border border-slate-700 px-3 py-2 text-sm font-semibold text-slate-100 transition hover:border-sky-400 hover:text-sky-200 disabled:cursor-not-allowed disabled:opacity-60"
        >
          {resetPassword.isPending ? 'Saving…' : 'Reset'}
        </button>
      </div>
      <FormError error={resetPassword.error} action="Reset password" />
    </form>
  );
}

function UserCard({ user, currentUserId }: { user: UserView; currentUserId: number | null }) {
  const deleteUser = useDeleteAdminUserMutation();
  const isSelf = currentUserId === user.id;

  return (
    <article className="rounded-2xl border border-slate-800 bg-slate-900/60 p-4">
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0">
          <h2 className="truncate text-base font-semibold text-slate-100">{user.email}</h2>
          <p className="mt-1 text-sm text-slate-400">{user.display_name || 'No display name'}</p>
        </div>
        <div className="flex shrink-0 items-center gap-2">
          {user.is_admin ? (
            <span className="rounded-full border border-sky-500/40 bg-sky-400/10 px-2 py-1 text-xs font-semibold text-sky-100">
              Admin
            </span>
          ) : null}
          <button
            type="button"
            onClick={() => deleteUser.mutate(user.id)}
            disabled={deleteUser.isPending || isSelf}
            title={isSelf ? 'You cannot delete your own admin account.' : undefined}
            className="rounded-lg border border-slate-700 px-3 py-1.5 text-xs font-semibold text-slate-100 transition hover:border-red-400 hover:text-red-100 disabled:cursor-not-allowed disabled:opacity-50"
          >
            {deleteUser.isPending ? 'Deleting…' : 'Delete'}
          </button>
        </div>
      </div>
      {isSelf ? <p className="mt-3 text-xs text-slate-500">Signed-in account cannot delete itself.</p> : null}
      <ResetPasswordForm user={user} />
      <FormError error={deleteUser.error} action="Delete user" />
    </article>
  );
}

function UsersSection() {
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
          <UserCard key={user.id} user={user} currentUserId={currentUser?.id ?? null} />
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
    <form onSubmit={onSubmit} className="rounded-2xl border border-slate-800 bg-slate-900/60 p-4">
      <h2 className="text-lg font-semibold text-slate-100">Add domain</h2>
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
          className="w-full rounded-lg bg-sky-400 px-4 py-2 text-sm font-semibold text-slate-950 transition hover:bg-sky-300 disabled:cursor-not-allowed disabled:opacity-60"
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
      <section className="rounded-2xl border border-slate-800 bg-slate-900/50 p-5">
        <h2 className="text-lg font-semibold text-slate-100">Domains</h2>
        {domains.isPending ? (
          <p className="mt-4 text-sm text-slate-400">Loading domains…</p>
        ) : domains.isError ? (
          <p role="alert" className="mt-4 text-sm text-red-200">
            {adminErrorMessage(domains.error, 'Load domains')}
          </p>
        ) : domains.data.domains.length === 0 ? (
          <p className="mt-4 text-sm text-slate-400">No domains configured yet.</p>
        ) : (
          <ul className="mt-4 space-y-2">
            {domains.data.domains.map((domain) => (
              <li key={domain} className="flex items-center justify-between gap-3 rounded-xl border border-slate-800 bg-slate-950/60 px-3 py-2">
                <span className="min-w-0 truncate font-medium text-slate-100">{domain}</span>
                <button
                  type="button"
                  onClick={() => deleteDomain.mutate(domain)}
                  disabled={deleteDomain.isPending}
                  className="rounded-lg border border-slate-700 px-3 py-1.5 text-xs font-semibold text-slate-100 transition hover:border-red-400 hover:text-red-100 disabled:cursor-not-allowed disabled:opacity-60"
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
        <div className="rounded-2xl border border-slate-800 bg-slate-900/50 p-6">
          <h2 className="text-lg font-semibold text-slate-100">403 Forbidden</h2>
          <p className="mt-3 text-sm leading-6 text-slate-400">
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
      list={<UsersSection />}
      reading={
        <div className="space-y-4">
          <section className="rounded-2xl border border-slate-800 bg-slate-900/50 p-5">
            <h2 className="text-lg font-semibold text-slate-100">Operator settings</h2>
            <p className="mt-3 text-sm leading-6 text-slate-400">
              User and domain operations call the hail API, which forwards changes to Stalwart management and refreshes these lists after each mutation.
            </p>
          </section>
          <DomainsSection />
        </div>
      }
    />
  );
}
