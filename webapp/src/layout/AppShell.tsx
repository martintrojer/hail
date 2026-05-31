import { Link, useLocation, useNavigate } from '@tanstack/react-router';
import { type ComponentType, type ReactNode, useMemo, useState } from 'react';
import type { LabelResponse, ViewCountsResponse } from '../api/client';
import { useApiClient } from '../api/ApiClientProvider';
import { useLabels, useViewCounts } from '../api/query';
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
  SlidersHorizontal,
  Sun,
  Tags,
  Trash2,
  UserRoundPlus,
} from '../components/icons';
import { Badge } from '../components/ui/badge';
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
  SidebarMenuBadge,
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
import { cn } from '../lib/utils';

export type AppShellContentLayout = 'list' | 'split' | 'reading' | 'composer' | 'wide';

const appShellContentWidthClasses: Record<AppShellContentLayout, string> = {
  list: 'max-w-none',
  split: 'max-w-none xl:max-w-7xl',
  reading: 'max-w-3xl lg:max-w-4xl xl:max-w-5xl',
  composer: 'max-w-3xl lg:max-w-4xl xl:max-w-5xl',
  wide: 'max-w-6xl',
};

export function appShellContentWidthClass(layout: AppShellContentLayout) {
  return appShellContentWidthClasses[layout];
}

function defaultContentLayout({
  list,
  reading,
  wide,
}: Pick<AppShellProps, 'list' | 'reading' | 'wide'>): AppShellContentLayout {
  if (wide) return 'wide';
  if (list && reading) return 'split';
  if (list) return 'list';
  return 'reading';
}

interface AppShellProps {
  title: string;
  description?: string;
  list?: ReactNode;
  reading?: ReactNode;
  actions?: ReactNode;
  /** Use wider content area (e.g. for two-column layouts). */
  wide?: boolean;
  /** Named content sizing variant; defaults from provided list/reading content. */
  contentLayout?: AppShellContentLayout;
}

interface NavItem {
  to: string;
  label: string;
  description: string;
  Icon: ComponentType<{ className?: string }>;
  countKey?: keyof ViewCountsResponse;
}

type SidebarCounts = Partial<ViewCountsResponse>;

const primaryNavItems: NavItem[] = [
  {
    to: '/imbox',
    label: 'Imbox',
    description: 'Important mail from approved people lands here.',
    Icon: Inbox,
    countKey: 'imbox_new',
  },
  {
    to: '/feed',
    label: 'The Feed',
    description: 'Newsletters and recurring reading can collect here.',
    Icon: Rss,
    countKey: 'feed_unread',
  },
  {
    to: '/papertrail',
    label: 'Paper Trail',
    description: 'Receipts, statements, and reference mail will land here.',
    Icon: ReceiptText,
    countKey: 'papertrail_unread',
  },
  {
    to: '/screener',
    label: 'The Screener',
    description: 'New senders end up here. Decide if they get in.',
    Icon: UserRoundPlus,
    countKey: 'screener_pending',
  },
];

const pileNavItems: NavItem[] = [
  {
    to: '/set-aside',
    label: 'Set Aside',
    description: 'Threads you want to keep handy without leaving them in the Imbox.',
    Icon: Bookmark,
    countKey: 'set_aside',
  },
  {
    to: '/reply-later',
    label: 'Reply Later',
    description: 'Threads waiting for a response when you have time.',
    Icon: Clock3,
    countKey: 'reply_later',
  },
  {
    to: '/bubble-up',
    label: 'Bubble Up',
    description: 'Threads scheduled to return to your attention.',
    Icon: ArrowUpCircle,
    countKey: 'bubble_up',
  },
];

const mailboxNavItems: NavItem[] = [
  {
    to: '/drafts',
    label: 'Drafts',
    description: 'Resume messages you started but have not sent yet.',
    Icon: FilePenLine,
    countKey: 'drafts',
  },
  {
    to: '/scheduled',
    label: 'Scheduled',
    description: 'Messages waiting for scheduled delivery.',
    Icon: Send,
    countKey: 'scheduled',
  },
  {
    to: '/archive',
    label: 'Archive',
    description: 'Mail you have dealt with and moved out of the Imbox.',
    Icon: Archive,
  },
  {
    to: '/spam',
    label: 'Spam',
    description: 'Mail identified as spam collects here until you restore or delete it.',
    Icon: ShieldAlert,
    countKey: 'spam',
  },
  {
    to: '/trash',
    label: 'Trash',
    description: 'Deleted mail stays here until it is permanently removed.',
    Icon: Trash2,
    countKey: 'trash',
  },
  {
    to: '/files',
    label: 'All Files',
    description: 'Every recent attachment in one place, with the mail thread it came from.',
    Icon: FileArchive,
  },
];

const workflowNavItems: NavItem[] = [
  {
    to: '/search',
    label: 'Search',
    description: 'Find mail and contact notes across hail.',
    Icon: Search,
  },
  {
    to: '/screener/speakeasy',
    label: 'Speakeasy Passphrase',
    description: 'Monthly password/passphrase bypass for one message at a time.',
    Icon: KeyRound,
  },
  {
    to: '/screened-out',
    label: 'Screened Out',
    description: 'Review blocked senders and allow mistakes into the right place.',
    Icon: ShieldOff,
  },
  {
    to: '/workflows',
    label: 'Workflows',
    description: 'Rules that classify, label, or auto-reply to incoming mail.',
    Icon: SlidersHorizontal,
  },
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

function focusedMailListThreadId() {
  if (!(document.activeElement instanceof HTMLElement)) {
    return null;
  }

  if (document.activeElement.dataset.hailMailListItem === 'true') {
    return document.activeElement.dataset.hailThreadId ?? null;
  }

  return null;
}

function dispatchFocusedMailShortcut(action: string) {
  if (!focusedMailListThreadId()) {
    return false;
  }

  dispatchMailShortcut(action);
  return true;
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

function itemCount(item: NavItem, counts: SidebarCounts) {
  return item.countKey ? counts[item.countKey] : undefined;
}

function CountPill({ count }: { count: number }) {
  const label = count > 99 ? '99+' : String(count);

  return (
    <Badge
      variant="secondary"
      className="ml-auto h-5 min-w-5 px-1 text-[0.7rem] tabular-nums group-data-[collapsible=icon]:absolute group-data-[collapsible=icon]:-right-1 group-data-[collapsible=icon]:-top-1 group-data-[collapsible=icon]:ml-0 group-data-[collapsible=icon]:h-4 group-data-[collapsible=icon]:min-w-4 group-data-[collapsible=icon]:px-1 group-data-[collapsible=icon]:text-[0.625rem]"
      aria-hidden="true"
    >
      {label}
    </Badge>
  );
}

function NavTooltip({ item, count }: { item: NavItem; count?: number }) {
  return (
    <div className="flex max-w-64 flex-col gap-1 text-left">
      <span className="font-medium">
        {item.label}
        {count && count > 0 ? ` (${count})` : ''}
      </span>
      <span className="text-xs opacity-85">{item.description}</span>
    </div>
  );
}

function NavMenuItem({
  item,
  activePath,
  counts,
}: {
  item: NavItem;
  activePath: string;
  counts: SidebarCounts;
}) {
  const isActive = activePath === item.to || activePath.startsWith(`${item.to}/`);
  const count = itemCount(item, counts);
  const showCount = count !== undefined && count > 0;
  const label = showCount ? `${item.label}, ${count} items` : item.label;

  return (
    <SidebarMenuItem>
      <SidebarMenuButton
        asChild
        isActive={isActive}
        tooltip={{ children: <NavTooltip item={item} count={count} />, hidden: false }}
      >
        <Link to={item.to} aria-label={label}>
          <item.Icon />
          <span>{item.label}</span>
          {showCount ? <CountPill count={count} /> : null}
        </Link>
      </SidebarMenuButton>
      {showCount ? (
        <SidebarMenuBadge className="sr-only">
          {count}
        </SidebarMenuBadge>
      ) : null}
    </SidebarMenuItem>
  );
}

function SidebarNavGroup({
  label,
  items,
  activePath,
  counts,
}: {
  label: string;
  items: NavItem[];
  activePath: string;
  counts: SidebarCounts;
}) {
  return (
    <SidebarGroup>
      <SidebarGroupLabel>{label}</SidebarGroupLabel>
      <SidebarGroupContent>
        <SidebarMenu>
          {items.map((item) => (
            <NavMenuItem key={item.to} item={item} activePath={activePath} counts={counts} />
          ))}
        </SidebarMenu>
      </SidebarGroupContent>
    </SidebarGroup>
  );
}

function labelSegments(label: LabelResponse) {
  const segments = label.path_segments.length > 0 ? label.path_segments : label.name.split('/');
  return segments.map((segment) => segment.trim()).filter(Boolean);
}

function labelDisplayPath(label: LabelResponse) {
  const segments = labelSegments(label);
  return segments.length > 0 ? segments.join(' / ') : label.name;
}

function labelDepth(label: LabelResponse) {
  return Math.max(labelSegments(label).length - 1, 0);
}

function sortLabelsForNav(labels: LabelResponse[]) {
  return [...labels].sort((left, right) => {
    const leftSegments = labelSegments(left).map((segment) => segment.toLocaleLowerCase());
    const rightSegments = labelSegments(right).map((segment) => segment.toLocaleLowerCase());
    const length = Math.max(leftSegments.length, rightSegments.length);

    for (let index = 0; index < length; index += 1) {
      const leftSegment = leftSegments[index];
      const rightSegment = rightSegments[index];

      if (leftSegment === undefined) {
        return -1;
      }
      if (rightSegment === undefined) {
        return 1;
      }
      const segmentOrder = leftSegment.localeCompare(rightSegment);
      if (segmentOrder !== 0) {
        return segmentOrder;
      }
    }

    return left.id - right.id;
  });
}

function LabelsNav({ activePath }: { activePath: string }) {
  const apiClient = useApiClient();
  const labelsQuery = useLabels(apiClient);
  const labels = useMemo(
    () => sortLabelsForNav(labelsQuery.data?.labels ?? []),
    [labelsQuery.data?.labels],
  );

  return (
    <SidebarGroup>
      <SidebarMenu>
        <SidebarMenuItem>
          <SidebarMenuButton
            asChild
            isActive={activePath === '/labels'}
            tooltip={{
              children: (
                <div className="flex max-w-64 flex-col gap-1 text-left">
                  <span className="font-medium">Labels</span>
                  <span className="text-xs opacity-85">Manage local thread labels.</span>
                </div>
              ),
              hidden: false,
            }}
          >
            <Link to="/labels">
              <Tags />
              <span>Manage labels</span>
            </Link>
          </SidebarMenuButton>
        </SidebarMenuItem>
      </SidebarMenu>
      <Collapsible defaultOpen>
        <SidebarGroupLabel asChild>
          <CollapsibleTrigger className="group/labels flex w-full items-center gap-2">
            <FolderOpen />
            <span>All labels</span>
            {labels.length > 0 ? (
              <Badge variant="secondary" className="ml-auto h-5 min-w-5 px-1 text-[0.7rem] tabular-nums">
                {labels.length}
              </Badge>
            ) : null}
            <ChevronDown className="transition-transform group-data-[state=open]/labels:rotate-180" />
          </CollapsibleTrigger>
        </SidebarGroupLabel>
        <CollapsibleContent>
          <SidebarMenuSub aria-label="All labels">
            {labelsQuery.isPending ? (
              <SidebarMenuSubItem>
                <span className="block px-2 py-1 text-xs text-muted-foreground">Loading labels…</span>
              </SidebarMenuSubItem>
            ) : labels.length === 0 ? (
              <SidebarMenuSubItem>
                <span className="block px-2 py-1 text-xs text-muted-foreground">No labels yet</span>
              </SidebarMenuSubItem>
            ) : (
              labels.map((label) => {
                const path = labelDisplayPath(label);
                const isActive = activePath === `/labels/${label.id}`;

                return (
                  <SidebarMenuSubItem key={label.id}>
                    <SidebarMenuSubButton
                      asChild
                      isActive={isActive}
                      title={`${label.name} · ${label.thread_count} ${label.thread_count === 1 ? 'thread' : 'threads'}`}
                      className="h-auto min-h-7"
                    >
                      <Link to="/labels/$labelId" params={{ labelId: String(label.id) }}>
                        <span
                          className="min-w-0 truncate"
                          style={{ paddingLeft: `${labelDepth(label) * 0.75}rem` }}
                        >
                          {path}
                        </span>
                        {label.thread_count > 0 ? (
                          <span className="ml-auto text-xs tabular-nums text-muted-foreground">
                            {label.thread_count}
                          </span>
                        ) : null}
                      </Link>
                    </SidebarMenuSubButton>
                  </SidebarMenuSubItem>
                );
              })
            )}
          </SidebarMenuSub>
        </CollapsibleContent>
      </Collapsible>
    </SidebarGroup>
  );
}

function AppSidebar({
  activePath,
  isAdmin,
  counts,
  logout,
  logoutLoading,
}: {
  activePath: string;
  isAdmin: boolean;
  counts: SidebarCounts;
  logout: () => void;
  logoutLoading: boolean;
}) {
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
        <SidebarNavGroup label="Mail" items={primaryNavItems} activePath={activePath} counts={counts} />
        <SidebarNavGroup label="Pile" items={pileNavItems} activePath={activePath} counts={counts} />
        <SidebarNavGroup label="Folders" items={mailboxNavItems} activePath={activePath} counts={counts} />
        <LabelsNav activePath={activePath} />
        <SidebarNavGroup label="Tools" items={workflowNavItems} activePath={activePath} counts={counts} />
      </SidebarContent>

      <SidebarFooter>
        <SidebarSeparator />
        <SidebarMenu>
          <NavMenuItem
            item={{
              to: '/provider-accounts',
              label: 'Provider Accounts',
              description: 'Connect Gmail for one-way provider import into hail.',
              Icon: KeyRound,
            }}
            activePath={activePath}
            counts={counts}
          />
          <NavMenuItem
            item={{
              to: '/preferences',
              label: 'Preferences',
              description: 'Privacy and display preferences.',
              Icon: SlidersHorizontal,
            }}
            activePath={activePath}
            counts={counts}
          />
          {isAdmin ? (
            <NavMenuItem
              item={{
                to: '/admin',
                label: 'Admin',
                description: 'Manage mailbox users and accepted mail domains.',
                Icon: Settings,
              }}
              activePath={activePath}
              counts={counts}
            />
          ) : null}
          <SidebarMenuItem>
            <SidebarMenuButton
              type="button"
              onClick={logout}
              disabled={logoutLoading}
              tooltip={{
                children: logoutLoading ? 'Signing out…' : 'Sign Out',
                hidden: false,
              }}
              aria-label={logoutLoading ? 'Signing out…' : 'Sign Out'}
            >
              <LogOut />
              <span>{logoutLoading ? 'Signing out…' : 'Sign Out'}</span>
            </SidebarMenuButton>
          </SidebarMenuItem>
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
  list,
  reading,
  actions,
  wide,
  contentLayout: requestedContentLayout,
}: AppShellProps) {
  const { user, logout, logoutLoading } = useAuth();
  const navigate = useNavigate();
  const location = useLocation();
  const [shortcutHelpOpen, setShortcutHelpOpen] = useState(false);
  const { theme, setTheme } = useTheme();
  const viewCounts = useViewCounts();
  const sidebarCounts: SidebarCounts = viewCounts.data ?? {};
  const hasContent = Boolean(list || reading);
  const contentLayout = requestedContentLayout ?? defaultContentLayout({ list, reading, wide });
  const contentWidthClass = appShellContentWidthClass(contentLayout);

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
    onReply: () => {
      if (!dispatchFocusedMailShortcut('reply')) {
        focusReplyBox();
      }
    },
    onReplyAll: () => {
      dispatchFocusedMailShortcut('reply-all');
    },
    onForward: () => {
      dispatchFocusedMailShortcut('forward');
    },
    onAddNote: () => {
      dispatchFocusedMailShortcut('add-note');
    },
    onOpenActionMenu: () => {
      dispatchFocusedMailShortcut('open-menu');
    },
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
        <AppSidebar
          activePath={location.pathname}
          isAdmin={Boolean(user?.is_admin)}
          counts={sidebarCounts}
          logout={logout}
          logoutLoading={logoutLoading}
        />
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

          <main className="min-w-0 flex-1 overflow-x-hidden px-4 py-5 sm:px-6 lg:px-8">
            <div
              data-testid="app-shell-content"
              data-hail-content-layout={contentLayout}
              className={cn('mx-auto w-full min-w-0', contentWidthClass)}
            >
              <h1 className="sr-only">{title}</h1>
              {actions ? (
                <div className="mb-4 flex justify-end border-b pb-3">
                  <div className="shrink-0">{actions}</div>
                </div>
              ) : null}

              <div className="flex flex-col gap-8">
                {list}
                {reading}
                {!hasContent ? <EmptyList title={title} /> : null}
              </div>
            </div>
          </main>
        </SidebarInset>
      </SidebarProvider>
    </TooltipProvider>
  );
}
