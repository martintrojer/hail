import {
  Archive,
  ArrowLeft,
  ArrowUpCircle,
  Bookmark,
  ChevronDown,
  Clock,
  Forward,
  LogOut,
  Mail,
  Menu,
  Monitor,
  Moon,
  MoreHorizontal,
  Paperclip,
  PenSquare,
  Plus,
  Reply,
  Search,
  Send,
  Settings,
  ShieldQuestion,
  StickyNote,
  Sun,
  Trash2,
  UserPlus,
  X,
  type LucideIcon,
} from "lucide-react";

export {
  Archive,
  ArrowLeft,
  ArrowUpCircle,
  Bookmark,
  ChevronDown,
  Clock,
  Forward,
  LogOut,
  Mail,
  Menu,
  Monitor,
  Moon,
  MoreHorizontal,
  Paperclip,
  PenSquare,
  Plus,
  Reply,
  Search,
  Send,
  Settings,
  ShieldQuestion,
  StickyNote,
  Sun,
  Trash2,
  UserPlus,
  X,
};
export type { LucideIcon };

export const iconSizes = {
  sm: 16,
  md: 18,
  lg: 20,
  xl: 24,
} as const;

export type IconSize = keyof typeof iconSizes;
export type IconSizePx = (typeof iconSizes)[IconSize];

export const iconClassNames = {
  default: "text-ink-secondary",
  interactive: "text-ink-secondary hover:text-ink-primary",
} as const;

export const iconStrokeWidth = 1.5 as const;

export const iconSizeProps = {
  sm: { size: iconSizes.sm, strokeWidth: iconStrokeWidth },
  md: { size: iconSizes.md, strokeWidth: iconStrokeWidth },
  lg: { size: iconSizes.lg, strokeWidth: iconStrokeWidth },
  xl: { size: iconSizes.xl, strokeWidth: iconStrokeWidth },
} as const satisfies Record<IconSize, { size: IconSizePx; strokeWidth: typeof iconStrokeWidth }>;

export const icons = {
  search: Search,
  screener: UserPlus,
  screenerShield: ShieldQuestion,
  more: MoreHorizontal,
  reply: Reply,
  forward: Forward,
  attach: Paperclip,
  compose: PenSquare,
  trash: Trash2,
  setAside: Bookmark,
  replyLater: Clock,
  bubbleUp: ArrowUpCircle,
  note: StickyNote,
  back: ArrowLeft,
  menu: Menu,
  close: X,
  chevronDown: ChevronDown,
  mail: Mail,
  send: Send,
  plus: Plus,
  settings: Settings,
  logOut: LogOut,
  monitor: Monitor,
  moon: Moon,
  sun: Sun,
} as const satisfies Record<string, LucideIcon>;
