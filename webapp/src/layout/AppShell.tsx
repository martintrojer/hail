import { Link, useNavigate } from '@tanstack/react-router';
import {
  useEffect,
  useRef,
  useState,
  type ComponentType,
  type ReactNode,
} from 'react';
import { useAuth } from '../auth/AuthProvider';
import { KeyboardShortcutHelp } from '../components/KeyboardShortcutHelp';
import {
  ArrowUpCircle,
  Bookmark,
  Clock,
  LogOut,
  Mail,
  PenSquare,
  Search,
  Settings,
  UserPlus,
  iconSizeProps,
} from '../components/icons';
import { useKeyboardShortcuts } from '../hooks/useKeyboardShortcuts';
import { Pile } from './Pile';

interface AppShellProps {
  title: string;
  description?: string;
  list?: ReactNode;
  reading?: ReactNode;
  actions?: ReactNode;
}

interface MenuItem {
  to: string;
  label: string;
  Icon: ComponentType<{ className?: string; size?: number; strokeWidth?: number }>;
}

const navItems: MenuItem[] = [
  { to: '/imbox', label: 'Imbox', Icon: Mail },
  { to: '/feed', label: 'The Feed', Icon: Mail },
  { to: '/papertrail', label: 'Paper Trail', Icon: Mail },
  { to: '/set-aside', label: 'Set Aside', Icon: Bookmark },
  { to: '/reply-later', label: 'Reply Later', Icon: Clock },
  { to: '/bubble-up', label: 'Bubble Up', Icon: ArrowUpCircle },
  { to: '/search', label: 'Search', Icon: Search },
];

function EmptyList({ title }: { title: string }) {
  return (
    <div className="flex min-h-64 flex-col items-center justify-center p-8 text-center">
      <p className="text-base font-semibold text-ink-primary">No mail here yet</p>
      <p className="mt-2 max-w-sm hail-preview">
        The {title} list will render here once the mail view endpoints are wired
        to the SPA.
      </p>
    </div>
  );
}

function userInitial(email: string | undefined) {
  const trimmed = email?.trim();
  if (!trimmed) {
    return 'H';
  }

  return trimmed.charAt(0).toUpperCase();
}

function focusSearchInput(attempt = 0) {
  window.setTimeout(() => {
    const input = document.querySelector<HTMLInputElement>(
      '[data-hail-search-input="true"]',
    );
    if (!input && attempt < 10) {
      focusSearchInput(attempt + 1);
      return;
    }

    input?.focus();
    input?.select();
  }, 25);
}

function focusMailListItem(direction: 1 | -1) {
  const items = Array.from(
    document.querySelectorAll<HTMLAnchorElement>('[data-hail-mail-list-item="true"]'),
  ).filter((item) => item.offsetParent !== null);

  if (items.length === 0) {
    return;
  }

  const activeIndex = items.findIndex((item) => item === document.activeElement);
  let nextIndex: number;
  if (activeIndex === -1) {
    nextIndex = direction === 1 ? 0 : items.length - 1;
  } else {
    nextIndex = Math.min(Math.max(activeIndex + direction, 0), items.length - 1);
  }

  items[nextIndex]?.focus();
}

function dispatchMailShortcut(action: string) {
  window.dispatchEvent(new CustomEvent('hail:mail-shortcut', { detail: { action } }));
}

function focusReplyBox() {
  const replyBox = document.querySelector<HTMLTextAreaElement>(
    '[data-hail-reply-box="true"]',
  );
  replyBox?.focus();
}

export function AppShell({
  title,
  description,
  list,
  reading,
  actions,
}: AppShellProps) {
  const { user, logout, logoutLoading } = useAuth();
  const navigate = useNavigate();
  const [menuOpen, setMenuOpen] = useState(false);
  const [shortcutHelpOpen, setShortcutHelpOpen] = useState(false);
  const menuRef = useRef<HTMLDivElement | null>(null);
  const logoButtonRef = useRef<HTMLButtonElement | null>(null);
  const hasContent = Boolean(list || reading);

  useKeyboardShortcuts({
    onNextThread: () => focusMailListItem(1),
    onPreviousThread: () => focusMailListItem(-1),
    onArchive: () => dispatchMailShortcut('archive'),
    onTrash: () => dispatchMailShortcut('trash'),
    onSetAside: () => dispatchMailShortcut('set-aside'),
    onReplyLater: () => dispatchMailShortcut('reply-later'),
    onReply: focusReplyBox,
    onCompose: () => void navigate({ to: '/compose' }),
    onFocusSearch: () => {
      void navigate({ to: '/search' });
      focusSearchInput();
    },
    onGoImbox: () => void navigate({ to: '/imbox' }),
    onGoFeed: () => void navigate({ to: '/feed' }),
    onGoPaperTrail: () => void navigate({ to: '/papertrail' }),
    onGoScreener: () => void navigate({ to: '/screener' }),
    onShowHelp: () => setShortcutHelpOpen(true),
  });

  useEffect(() => {
    if (!menuOpen) {
      return undefined;
    }

    function onPointerDown(event: PointerEvent) {
      const target = event.target as Node;
      if (
        menuRef.current?.contains(target) ||
        logoButtonRef.current?.contains(target)
      ) {
        return;
      }
      setMenuOpen(false);
    }

    function onKeyDown(event: KeyboardEvent) {
      if (event.key === 'Escape') {
        setMenuOpen(false);
        logoButtonRef.current?.focus();
      }
    }

    document.addEventListener('pointerdown', onPointerDown);
    document.addEventListener('keydown', onKeyDown);

    return () => {
      document.removeEventListener('pointerdown', onPointerDown);
      document.removeEventListener('keydown', onKeyDown);
    };
  }, [menuOpen]);

  function closeMenu() {
    setMenuOpen(false);
  }

  return (
    <div className="min-h-screen bg-bg-page text-ink-primary">
      <KeyboardShortcutHelp
        open={shortcutHelpOpen}
        onClose={() => setShortcutHelpOpen(false)}
      />
      {menuOpen ? (
        <button
          type="button"
          aria-label="Close main menu"
          className="fixed inset-0 z-30 cursor-default bg-ink-primary/5"
          onClick={closeMenu}
          tabIndex={-1}
        />
      ) : null}

      <header className="relative z-40 px-4 py-5 sm:px-6 sm:py-6">
        <div className="grid grid-cols-[auto_minmax(0,1fr)_auto] items-center gap-4 sm:gap-6">
          <div className="relative">
            <button
              ref={logoButtonRef}
              type="button"
              aria-haspopup="menu"
              aria-expanded={menuOpen}
              onClick={() => setMenuOpen((open) => !open)}
              className="rounded-md px-1 py-1 text-xl font-bold lowercase tracking-tight text-ink-primary outline-none hover:text-accent-blue focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent-blue"
            >
              hail
            </button>

            {menuOpen ? (
              <div
                ref={menuRef}
                role="menu"
                aria-label="Main menu"
                className="fixed inset-x-0 top-16 z-50 rounded-none border-y border-border-menu bg-bg-surface p-3 shadow-md shadow-ink-primary/15 sm:absolute sm:inset-x-auto sm:left-0 sm:top-12 sm:w-80 sm:rounded-lg sm:border"
              >
                <nav className="space-y-1" aria-label="Primary navigation">
                  {navItems.map(({ to, label, Icon }) => (
                    <Link
                      key={to}
                      to={to}
                      role="menuitem"
                      onClick={closeMenu}
                      className="flex items-center gap-3 rounded-md px-3 py-2.5 hail-chrome text-ink-secondary outline-none hover:bg-bg-hover hover:text-ink-primary focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent-blue"
                      activeProps={{
                        className: 'bg-bg-selected font-semibold text-ink-primary',
                      }}
                    >
                      <Icon className="shrink-0" {...iconSizeProps.sm} />
                      <span>{label}</span>
                    </Link>
                  ))}
                </nav>

                <Link
                  to="/compose"
                  role="menuitem"
                  onClick={closeMenu}
                  className="mt-3 flex items-center justify-center gap-2 rounded-lg bg-accent-blue px-3 py-2.5 hail-chrome font-semibold text-white outline-none hover:bg-accent-blue-hover focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent-blue"
                >
                  <PenSquare {...iconSizeProps.sm} />
                  Compose
                </Link>

                {user?.is_admin ? (
                  <Link
                    to="/admin"
                    role="menuitem"
                    onClick={closeMenu}
                    className="mt-2 flex items-center gap-3 rounded-md px-3 py-2.5 hail-chrome text-ink-secondary outline-none hover:bg-bg-hover hover:text-ink-primary focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent-blue"
                    activeProps={{
                      className: 'bg-bg-selected font-semibold text-ink-primary',
                    }}
                  >
                    <Settings className="shrink-0" {...iconSizeProps.sm} />
                    <span>Admin</span>
                  </Link>
                ) : null}

                <div className="mt-3 border-t border-border-hairline pt-3">
                  <p className="truncate px-3 hail-preview text-ink-secondary">
                    {user?.email ?? 'Signed in'}
                  </p>
                  <button
                    type="button"
                    role="menuitem"
                    onClick={() => {
                      closeMenu();
                      logout();
                    }}
                    disabled={logoutLoading}
                    className="mt-1 flex w-full items-center gap-3 rounded-md px-3 py-2.5 text-left hail-chrome text-ink-secondary outline-none hover:bg-bg-hover hover:text-ink-primary focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent-blue disabled:cursor-not-allowed disabled:opacity-60"
                  >
                    <LogOut className="shrink-0" {...iconSizeProps.sm} />
                    <span>{logoutLoading ? 'Signing out…' : 'Sign Out'}</span>
                  </button>
                </div>
              </div>
            ) : null}
          </div>

          <h1 className="min-w-0 truncate hail-page-title text-ink-primary">
            {title}
          </h1>

          <div className="flex shrink-0 items-center gap-2 sm:gap-3">
            <Link
              to="/search"
              aria-label="Search"
              className="rounded-full p-2 text-ink-secondary outline-none hover:bg-bg-hover hover:text-ink-primary focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent-blue"
            >
              <Search {...iconSizeProps.lg} />
            </Link>
            <Link
              to="/screener"
              aria-label="The Screener"
              className="relative rounded-full p-2 text-ink-secondary outline-none hover:bg-bg-hover hover:text-ink-primary focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent-blue"
            >
              <UserPlus {...iconSizeProps.lg} />
              <span className="sr-only">Screener</span>
            </Link>
            <div
              className="grid h-9 w-9 place-items-center rounded-full bg-bg-selected text-sm font-semibold text-ink-primary"
              title={user?.email ?? undefined}
            >
              <span aria-hidden="true">{userInitial(user?.email)}</span>
              <span className="sr-only">
                {user?.email ? `Signed in as ${user.email}` : 'Signed in'}
              </span>
            </div>
          </div>
        </div>
      </header>

      <main className="mx-auto w-full max-w-center-column px-4 pb-16 pt-2 sm:px-6">
        {description ? (
          <p className="mb-6 max-w-2xl hail-body text-ink-secondary">
            {description}
          </p>
        ) : null}
        {actions ? <div className="mb-6">{actions}</div> : null}

        <div className="space-y-8">
          {list}
          {reading}
          {!hasContent ? <EmptyList title={title} /> : null}
        </div>
      </main>
      <Pile />
    </div>
  );
}
