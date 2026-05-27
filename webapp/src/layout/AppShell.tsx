import { Link, useLocation, useNavigate } from '@tanstack/react-router';
import { type ComponentType, type ReactNode, useState } from 'react';
import { useAuth } from '../auth/AuthProvider';
import { KeyboardShortcutHelp } from '../components/KeyboardShortcutHelp';
import {
  Archive,
  ArrowUpCircle,
  Bookmark,
  ChevronDown,
  Clock3,
  FileArchive,
  FilePenLine,
  FolderOpen,
  Inbox,
  KeyRound,
  LogOut,
  Menu,
  Moon,
  Monitor,
  PenSquare,
  ReceiptText,
  Rss,
  Search,
  Send,
  Settings,
  ShieldAlert,
  ShieldOff,
  Sun,
  Tags,
  Trash2,
  UserRoundPlus,
} from '../components/icons';
import { Button } from '../components/ui/button';
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from '../components/ui/collapsible';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from '../components/ui/dropdown-menu';
import {
  Sidebar,
  SidebarContent,
  SidebarFooter,
  SidebarGroup,
  SidebarGroupContent,
  SidebarGroupLabel,
  SidebarHeader,
  SidebarInset,
  SidebarMenu,
  SidebarMenuButton,
  SidebarMenuItem,
  SidebarMenuSub,
  SidebarMenuSubButton,
  SidebarMenuSubItem,
  SidebarProvider,
  SidebarRail,
  SidebarSeparator,
  SidebarTrigger,
} from '../components/ui/sidebar';
import { TooltipProvider } from '../components/ui/tooltip';
import { useKeyboardShortcuts } from '../hooks/useKeyboardShortcuts';
import { useTheme, type ThemePreference } from '../hooks/useTheme';
import { Pile } from './Pile';

interface AppShellProps {
  title: string;
  description?: string;
  list?: ReactNode;
  reading?: ReactNode;
  actions?: ReactNode;
  /** Use wider content area (e.g. for two-column layouts). */
  wide?: boolean;
}

interface NavItem {
  to: string;
  label: string;
  Icon: ComponentType<{ className?: string }>;
}

const primaryNavItems: NavItem[] = [
  { to: '/imbox', label: 'Imbox', Icon: Inbox },
  { to: '/feed', label: 'The Feed', Icon: Rss },
  { to: '/papertrail', label: 'Paper Trail', Icon: ReceiptText },
  { to: '/screener', label: 'The Screener', Icon: UserRoundPlus },
];

const pileNavItems: NavItem[] = [
  { to: '/set-aside', label: 'Set Aside', Icon: Bookmark },
  { to: '/reply-later', label: 'Reply Later', Icon: Clock3 },
  { to: '/bubble-up', label: 'Bubble Up', Icon: ArrowUpCircle },
];

const mailboxNavItems: NavItem[] = [
  { to: '/drafts', label: 'Drafts', Icon: FilePenLine },
  { to: '/scheduled', label: 'Scheduled', Icon: Send },
  { to: '/archive', label: 'Archive', Icon: Archive },
  { to: '/spam', label: 'Spam', Icon: ShieldAlert },
  { to: '/trash', label: 'Trash', Icon: Trash2 },
  { to: '/files', label: 'All Files', Icon: FileArchive },
];

const workflowNavItems: NavItem[] = [
  { to: '/search', label: 'Search', Icon: Search },
  { to: '/screened-out', label: 'Screened Out', Icon: ShieldOff },
];

function EmptyList({ title }: { title: string }) {
  return (
    <div className="flex min-h-64 flex-col items-center justify-center p-8 text-center">
      <p className="text-base font-semibold text-foreground">No mail here yet</p>
      <p className="mt-2 max-w-sm text-sm leading-6 text-muted-foreground">
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

function focusFirstMailListItem() {
  const items = Array.from(
    document.querySelectorAll<HTMLAnchorElement>('[data-hail-mail-list-item="true"]'),
  ).filter((item) => item.offsetParent !== null);
  items[0]?.focus();
}

function focusLastMailListItem() {
  const items = Array.from(
    document.querySelectorAll<HTMLAnchorElement>('[data-hail-mail-list-item="true"]'),
  ).filter((item) => item.offsetParent !== null);
  items.at(-1)?.focus();
}

function openFocusedMailListItem() {
  const focused = document.activeElement;
  if (focused instanceof HTMLAnchorElement && focused.dataset.hailMailListItem === 'true') {
    focused.click();
  }
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

const nextTheme: Record<ThemePreference, ThemePreference> = {
  system: 'light',
  light: 'dark',
  dark: 'system',
};

const themeLabels: Record<ThemePreference, string> = {
  system: 'System theme',
  light: 'Light theme',
  dark: 'Dark theme',
};

function ThemeIcon({ theme }: { theme: ThemePreference }) {
  if (theme === 'light') {
    return <Sun />;
  }

  if (theme === 'dark') {
    return <Moon />;
  }

  return <Monitor />;
}

function NavMenuItem({ item, activePath }: { item: NavItem; activePath: string }) {
  const isActive = activePath === item.to || activePath.startsWith(`${item.to}/`);

  return (
    <SidebarMenuItem>
      <SidebarMenuButton asChild isActive={isActive} tooltip={item.label}>
        <Link to={item.to}>
          <item.Icon />
          <span>{item.label}</span>
        </Link>
      </SidebarMenuButton>
    </SidebarMenuItem>
  );
}

function SidebarNavGroup({
  label,
  items,
  activePath,
}: {
  label: string;
  items: NavItem[];
  activePath: string;
}) {
  return (
    <SidebarGroup>
      <SidebarGroupLabel>{label}</SidebarGroupLabel>
      <SidebarGroupContent>
        <SidebarMenu>
          {items.map((item) => (
            <NavMenuItem key={item.to} item={item} activePath={activePath} />
          ))}
        </SidebarMenu>
      </SidebarGroupContent>
    </SidebarGroup>
  );
}

function LabelsNavPlaceholder() {
  return (
    <SidebarGroup>
      <SidebarMenu>
        <SidebarMenuItem>
          <SidebarMenuButton asChild tooltip="Labels">
            <Link to="/search" search={{}}>
              <Tags />
              <span>Labels</span>
            </Link>
          </SidebarMenuButton>
        </SidebarMenuItem>
      </SidebarMenu>
      <Collapsible defaultOpen>
        <SidebarGroupLabel asChild>
          <CollapsibleTrigger className="group/labels flex w-full items-center gap-2">
            <FolderOpen />
            <span>All labels</span>
            <ChevronDown className="ml-auto transition-transform group-data-[state=open]/labels:rotate-180" />
          </CollapsibleTrigger>
        </SidebarGroupLabel>
        <CollapsibleContent>
          <SidebarMenuSub>
            <SidebarMenuSubItem>
              <SidebarMenuSubButton asChild>
                <Link to="/search" search={{}}>
                  <span>Uncategorized</span>
                </Link>
              </SidebarMenuSubButton>
            </SidebarMenuSubItem>
          </SidebarMenuSub>
        </CollapsibleContent>
      </Collapsible>
    </SidebarGroup>
  );
}

function AppSidebar({ activePath, isAdmin }: { activePath: string; isAdmin: boolean }) {
  return (
    <Sidebar collapsible="icon" className="border-sidebar-border bg-sidebar text-sidebar-foreground">
      <SidebarHeader>
        <SidebarMenu>
          <SidebarMenuItem>
            <SidebarMenuButton asChild size="lg" tooltip="hail">
              <Link to="/imbox" className="font-semibold" aria-label="hail Imbox">
                <span className="grid size-7 shrink-0 place-items-center rounded-md bg-sidebar-accent text-sidebar-accent-foreground ring-1 ring-sidebar-border">
                  <img
                    src="/logo-icon-transparent.png"
                    alt=""
                    className="size-5 object-contain"
                    aria-hidden="true"
                  />
                </span>
                <span className="truncate">hail</span>
                <span className="sr-only"> menu</span>
              </Link>
            </SidebarMenuButton>
          </SidebarMenuItem>
        </SidebarMenu>
      </SidebarHeader>

      <SidebarContent>
        <SidebarNavGroup label="Mail" items={primaryNavItems} activePath={activePath} />
        <SidebarNavGroup label="Pile" items={pileNavItems} activePath={activePath} />
        <SidebarNavGroup label="Folders" items={mailboxNavItems} activePath={activePath} />
        <LabelsNavPlaceholder />
        <SidebarNavGroup label="Tools" items={workflowNavItems} activePath={activePath} />
      </SidebarContent>

      <SidebarFooter>
        <SidebarSeparator />
        <SidebarMenu>
          <NavMenuItem
            item={{ to: '/provider-accounts', label: 'Provider Accounts', Icon: KeyRound }}
            activePath={activePath}
          />
          {isAdmin ? (
            <NavMenuItem
              item={{ to: '/admin', label: 'Admin', Icon: Settings }}
              activePath={activePath}
            />
          ) : null}
        </SidebarMenu>
      </SidebarFooter>
      <SidebarRail />
    </Sidebar>
  );
}

function UserMenu({
  email,
  logout,
  logoutLoading,
}: {
  email: string | undefined;
  logout: () => void;
  logoutLoading: boolean;
}) {
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button variant="ghost" size="icon" aria-label="hail account menu" title={email}>
          <span className="grid size-7 place-items-center rounded-full bg-muted text-xs font-semibold text-muted-foreground">
            {userInitial(email)}
          </span>
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="w-56">
        <DropdownMenuLabel className="truncate">
          {email ?? 'Signed in'}
        </DropdownMenuLabel>
        <DropdownMenuSeparator />
        <DropdownMenuGroup>
          <DropdownMenuItem
            disabled={logoutLoading}
            onSelect={() => {
              logout();
            }}
          >
            <LogOut />
            <span>{logoutLoading ? 'Signing out…' : 'Sign Out'}</span>
          </DropdownMenuItem>
        </DropdownMenuGroup>
      </DropdownMenuContent>
      {email ? <span className="sr-only">{email}</span> : null}
    </DropdownMenu>
  );
}

export function AppShell({
  title,
  description,
  list,
  reading,
  actions,
  wide,
}: AppShellProps) {
  const { user, logout, logoutLoading } = useAuth();
  const navigate = useNavigate();
  const location = useLocation();
  const [shortcutHelpOpen, setShortcutHelpOpen] = useState(false);
  const { theme, setTheme } = useTheme();
  const hasContent = Boolean(list || reading);

  useKeyboardShortcuts({
    onNextThread: () => focusMailListItem(1),
    onPreviousThread: () => focusMailListItem(-1),
    onFirstThread: focusFirstMailListItem,
    onLastThread: focusLastMailListItem,
    onHalfPageDown: () => { window.scrollBy({ top: window.innerHeight / 2, behavior: 'smooth' }); },
    onHalfPageUp: () => { window.scrollBy({ top: -window.innerHeight / 2, behavior: 'smooth' }); },
    onOpenThread: openFocusedMailListItem,
    onArchive: () => dispatchMailShortcut('archive'),
    onTrash: () => dispatchMailShortcut('trash'),
    onSetAside: () => dispatchMailShortcut('set-aside'),
    onReplyLater: () => dispatchMailShortcut('reply-later'),
    onReply: focusReplyBox,
    onCompose: () => void navigate({ to: '/compose', search: {} }),
    onFocusSearch: () => {
      void navigate({ to: '/search' });
      focusSearchInput();
    },
    onGoImbox: () => void navigate({ to: '/imbox' }),
    onGoFeed: () => void navigate({ to: '/feed' }),
    onGoPaperTrail: () => void navigate({ to: '/papertrail' }),
    onGoScreener: () => void navigate({ to: '/screener' }),
    onGoDrafts: () => void navigate({ to: '/drafts' }),
    onGoSpam: () => void navigate({ to: '/spam' }),
    onGoTrash: () => void navigate({ to: '/trash' }),
    onGoSetAside: () => void navigate({ to: '/set-aside' }),
    onGoReplyLater: () => void navigate({ to: '/reply-later' }),
    onGoBubbleUp: () => void navigate({ to: '/bubble-up' }),
    onGoArchive: () => void navigate({ to: '/archive' }),
    onToggleMenu: () => {
      window.dispatchEvent(
        new KeyboardEvent('keydown', { key: 'b', ctrlKey: true, bubbles: true }),
      );
    },
    onShowHelp: () => setShortcutHelpOpen(true),
    onEscape: () => {
      if (shortcutHelpOpen) {
        setShortcutHelpOpen(false);
      }
    },
  });

  return (
    <TooltipProvider>
      <SidebarProvider>
        <KeyboardShortcutHelp
          open={shortcutHelpOpen}
          onClose={() => setShortcutHelpOpen(false)}
        />
        <AppSidebar activePath={location.pathname} isAdmin={Boolean(user?.is_admin)} />
        <SidebarInset>
            <header className="sticky top-0 z-20 flex h-12 shrink-0 items-center gap-2 border-b bg-background/95 px-3 backdrop-blur supports-backdrop-filter:bg-background/80">
            <SidebarTrigger aria-label="Toggle navigation">
              <Menu />
            </SidebarTrigger>
            <Button variant="ghost" size="sm" asChild className="hidden sm:inline-flex">
              <Link to="/search">
                <Search data-icon="inline-start" />
                Search
              </Link>
            </Button>
            <div className="min-w-0 flex-1" />
            <Button size="sm" asChild>
              <Link to="/compose" search={{}}>
                <PenSquare data-icon="inline-start" />
                <span className="hidden sm:inline">Compose</span>
              </Link>
            </Button>
            <Button
              type="button"
              variant="ghost"
              size="icon-sm"
              aria-label={`${themeLabels[theme]}; switch to ${themeLabels[nextTheme[theme]].toLowerCase()}`}
              title={themeLabels[theme]}
              onClick={() => setTheme(nextTheme[theme])}
            >
              <ThemeIcon theme={theme} />
            </Button>
            <UserMenu
              email={user?.email}
              logout={logout}
              logoutLoading={logoutLoading}
            />
          </header>

          <main className="flex-1 px-4 py-5 sm:px-6 lg:px-8">
            <div className={wide ? 'mx-auto w-full max-w-6xl' : 'mx-auto w-full max-w-4xl'}>
              <div className="mb-5 flex flex-col gap-3 border-b pb-4 sm:flex-row sm:items-end sm:justify-between">
                <div className="min-w-0">
                  <h1 className="truncate text-2xl font-semibold tracking-tight text-foreground sm:text-3xl">
                    {title}
                  </h1>
                  {description ? (
                    <p className="mt-1 max-w-2xl text-sm text-muted-foreground">
                      {description}
                    </p>
                  ) : null}
                </div>
                {actions ? <div className="shrink-0">{actions}</div> : null}
              </div>

              <div className="flex flex-col gap-8">
                {list}
                {reading}
                {!hasContent ? <EmptyList title={title} /> : null}
              </div>
            </div>
          </main>
        </SidebarInset>
        <Pile />
      </SidebarProvider>
    </TooltipProvider>
  );
}
