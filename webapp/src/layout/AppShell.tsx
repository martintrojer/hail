import type { ReactNode } from 'react';
import { Link } from '@tanstack/react-router';
import { useAuth } from '../auth/AuthProvider';
import { Pile } from './Pile';

const navItems = [
  { label: 'Imbox', to: '/imbox' },
  { label: 'Feed', to: '/feed' },
  { label: 'Paper Trail', to: '/papertrail' },
  { label: 'Screener', to: '/screener' },
  { label: 'Set Aside', to: '/set-aside' },
  { label: 'Reply Later', to: '/reply-later' },
  { label: 'Search', to: '/search' },
] as const;

interface AppShellProps {
  title: string;
  description?: string;
  list?: ReactNode;
  reading?: ReactNode;
  actions?: ReactNode;
}

function EmptyList({ title }: { title: string }) {
  return (
    <div className="flex min-h-64 flex-col items-center justify-center rounded-2xl border border-dashed border-slate-800 bg-slate-900/40 p-8 text-center">
      <p className="text-base font-semibold text-slate-200">No mail here yet</p>
      <p className="mt-2 max-w-sm text-sm text-slate-400">
        The {title} list will render here once the mail view endpoints are wired
        to the SPA.
      </p>
    </div>
  );
}

function ReadingPlaceholder() {
  return (
    <div className="flex min-h-80 flex-col items-center justify-center rounded-2xl border border-dashed border-slate-800 bg-slate-900/40 p-8 text-center lg:min-h-full">
      <p className="text-base font-semibold text-slate-200">
        Select a thread
      </p>
      <p className="mt-2 max-w-sm text-sm text-slate-400">
        Thread-as-document rendering will appear in this reading pane.
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

  return (
    <div className="min-h-screen bg-slate-950 text-slate-50 lg:h-screen lg:overflow-hidden">
      <div className="grid min-h-screen grid-cols-1 lg:h-screen lg:grid-cols-[16rem_minmax(18rem,28rem)_minmax(0,1fr)]">
        <aside className="flex border-b border-slate-800 bg-slate-950/95 p-4 lg:min-h-0 lg:flex-col lg:border-b-0 lg:border-r">
          <div className="flex w-full flex-col gap-4">
            <div>
              <p className="text-xs font-medium uppercase tracking-[0.35em] text-sky-300">
                hail
              </p>
              <p className="mt-2 truncate text-sm text-slate-400">
                {user?.email ?? 'Signed in'}
              </p>
            </div>

            <nav aria-label="Mailbox views" className="flex flex-wrap gap-2 lg:flex-col">
              {navItems.map((item) => (
                <Link
                  key={item.to}
                  to={item.to}
                  activeProps={{
                    className: 'border-sky-400 bg-sky-400/10 text-sky-100',
                  }}
                  inactiveProps={{
                    className:
                      'border-transparent text-slate-300 hover:border-slate-700 hover:bg-slate-900 hover:text-slate-50',
                  }}
                  className="rounded-lg border px-3 py-2 text-sm font-medium transition"
                >
                  {item.label}
                </Link>
              ))}
              {user?.is_admin ? (
                <Link
                  to="/admin"
                  activeProps={{
                    className: 'border-sky-400 bg-sky-400/10 text-sky-100',
                  }}
                  inactiveProps={{
                    className:
                      'border-transparent text-slate-300 hover:border-slate-700 hover:bg-slate-900 hover:text-slate-50',
                  }}
                  className="rounded-lg border px-3 py-2 text-sm font-medium transition"
                >
                  Admin
                </Link>
              ) : null}
            </nav>
          </div>

          <div className="mt-4 hidden border-t border-slate-800 pt-4 lg:block">
            <p className="truncate text-sm font-medium text-slate-200">
              {user?.email}
            </p>
            <button
              type="button"
              onClick={logout}
              disabled={logoutLoading}
              className="mt-3 w-full rounded-lg border border-slate-700 px-3 py-2 text-sm font-semibold text-slate-100 transition hover:border-sky-400 hover:text-sky-200 disabled:cursor-not-allowed disabled:opacity-60"
            >
              {logoutLoading ? 'Signing out…' : 'Logout'}
            </button>
          </div>

          <button
            type="button"
            onClick={logout}
            disabled={logoutLoading}
            className="ml-4 self-start rounded-lg border border-slate-700 px-3 py-2 text-sm font-semibold text-slate-100 transition hover:border-sky-400 hover:text-sky-200 disabled:cursor-not-allowed disabled:opacity-60 lg:hidden"
          >
            {logoutLoading ? 'Signing out…' : 'Logout'}
          </button>
        </aside>

        <section className="border-b border-slate-800 bg-slate-950 p-4 lg:min-h-0 lg:overflow-y-auto lg:border-b-0 lg:border-r">
          <header className="mb-4 flex items-start justify-between gap-4">
            <div>
              <h1 className="text-3xl font-semibold tracking-tight">{title}</h1>
              {description ? (
                <p className="mt-2 text-sm text-slate-400">{description}</p>
              ) : null}
            </div>
            {actions ? <div className="shrink-0">{actions}</div> : null}
          </header>
          {list ?? <EmptyList title={title} />}
        </section>

        <main className="bg-slate-950 p-4 lg:min-h-0 lg:overflow-y-auto">
          {reading ?? <ReadingPlaceholder />}
        </main>
      </div>
      <Pile />
    </div>
  );
}
