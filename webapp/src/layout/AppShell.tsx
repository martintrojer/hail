import type { ReactNode } from 'react';
import { useAuth } from '../auth/AuthProvider';
import { Pile } from './Pile';

interface AppShellProps {
  title: string;
  description?: string;
  list?: ReactNode;
  reading?: ReactNode;
  actions?: ReactNode;
}

function EmptyList({ title }: { title: string }) {
  return (
    <div className="flex min-h-64 flex-col items-center justify-center border border-dashed border-border-hairline bg-bg-banner p-8 text-center">
      <p className="text-base font-semibold text-ink-primary">No mail here yet</p>
      <p className="mt-2 max-w-sm text-sm text-ink-secondary">
        The {title} list will render here once the mail view endpoints are wired
        to the SPA.
      </p>
    </div>
  );
}

export function AppShell({
  title,
  description,
  list,
  reading,
  actions,
}: AppShellProps) {
  const { user, logout, logoutLoading } = useAuth();
  const hasContent = Boolean(list || reading);

  return (
    <div className="min-h-screen bg-bg-page text-ink-primary">
      <div className="mx-auto flex min-h-screen w-full max-w-center-column flex-col px-4 py-6 md:px-6 md:py-8">
        <header className="mb-8 flex items-start justify-between gap-4">
          <div className="min-w-0">
            <div className="flex items-baseline gap-4">
              <p className="shrink-0 text-sm font-bold lowercase tracking-tight text-ink-primary">
                hail
              </p>
              <h1 className="text-4xl font-bold leading-tight tracking-[-0.01em] text-ink-primary sm:text-[2.5rem]">
                {title}
              </h1>
            </div>
            {description ? (
              <p className="mt-2 text-sm leading-6 text-ink-secondary">
                {description}
              </p>
            ) : null}
            <p className="mt-2 truncate text-xs text-ink-tertiary">
              {user?.email ?? 'Signed in'}
            </p>
          </div>

          <div className="flex shrink-0 items-center gap-3">
            {actions ? <div className="shrink-0">{actions}</div> : null}
            <button
              type="button"
              onClick={logout}
              disabled={logoutLoading}
              className="rounded-lg border border-border-menu bg-bg-surface px-3 py-2 text-sm font-medium text-ink-secondary hover:border-accent-blue hover:text-accent-blue disabled:cursor-not-allowed disabled:opacity-60"
            >
              {logoutLoading ? 'Signing out…' : 'Logout'}
            </button>
          </div>
        </header>

        <main className="min-w-0 flex-1 space-y-8">
          {list}
          {reading}
          {!hasContent ? <EmptyList title={title} /> : null}
        </main>
      </div>
      <Pile />
    </div>
  );
}
