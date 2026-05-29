import type { ReactNode } from 'react';
import {
  Archive,
  Bookmark,
  Clock,
  Forward,
  MailOpen,
  MailCheck,
  Reply,
  StickyNote,
  Trash2,
  X,
  icons,
  type LucideIcon,
} from './icons';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from './ui/dropdown-menu';

export interface MessageActionPopupProps {
  open: boolean;
  onClose: () => void;
  onOpenChange?: (open: boolean) => void;
  onAction: (action: string, payload?: unknown) => void;
  /** Optional trigger lets Radix anchor the menu to the shadcn Button. */
  trigger?: ReactNode;
  /** Actions to hide based on context (e.g. hide bubble-up in pile views). */
  hiddenActions?: string[];
  /** Optional actions to append for list-row state toggles. */
  extraActions?: ActionItem[];
}

export interface ActionItem {
  action: string;
  label: string;
  icon: LucideIcon;
  payload?: unknown;
  variant?: 'default' | 'destructive';
}

const actionGroups: ActionItem[][] = [
  [
    { action: 'reply', label: 'Reply', icon: Reply },
    { action: 'reply-all', label: 'Reply All', icon: Reply },
    { action: 'forward', label: 'Forward', icon: Forward },
  ],
  [
    { action: 'archive', label: 'Archive', icon: Archive },
    { action: 'set-aside', label: 'Set Aside', icon: Bookmark },
    { action: 'reply-later', label: 'Reply Later', icon: Clock },
    { action: 'bubble-up', label: 'Bubble Up', icon: icons.bubbleUp },
  ],
  [{ action: 'add-note', label: 'Add a Note', icon: StickyNote }],
  [
    { action: 'mark-spam', label: 'Mark as spam', icon: X, variant: 'destructive' },
    { action: 'trash', label: 'Trash', icon: Trash2, variant: 'destructive' },
  ],
];

export const markReadAction: ActionItem = { action: 'mark-read', label: 'Mark read', icon: MailOpen };
export const markUnreadAction: ActionItem = { action: 'mark-unread', label: 'Mark unread', icon: MailCheck };

const moveTargets = [
  { label: 'Imbox', value: 'imbox' },
  { label: 'Feed', value: 'feed' },
  { label: 'Paper Trail', value: 'papertrail' },
] as const;

export function MessageActionPopup({
  open,
  onClose,
  onAction,
  onOpenChange,
  trigger,
  hiddenActions = [],
  extraActions = [],
}: MessageActionPopupProps) {
  function runAction(action: string, payload?: unknown) {
    onAction(action, payload);
  }

  const hidden = new Set(hiddenActions);
  const filteredGroups = actionGroups.map((group) =>
    group.filter((item) => !hidden.has(item.action)),
  );
  const filteredExtraActions = extraActions.filter((item) => !hidden.has(item.action));

  return (
    <DropdownMenu
      modal={false}
      open={open}
      onOpenChange={(nextOpen) => {
        onOpenChange?.(nextOpen);
        if (!nextOpen) {
          onClose();
        }
      }}
    >
      {trigger ? <DropdownMenuTrigger asChild>{trigger}</DropdownMenuTrigger> : null}
      <DropdownMenuContent align="end" aria-label="Message actions" className="w-56">
        <DropdownMenuGroup>
          {filteredGroups[0].map((item) => (
            <MessageMenuItem
              key={item.action}
              item={item}
              onSelect={() => runAction(item.action, item.payload)}
            />
          ))}
        </DropdownMenuGroup>

        <DropdownMenuSeparator />

        <DropdownMenuGroup>
          {filteredGroups[1].map((item) => (
            <MessageMenuItem
              key={item.action}
              item={item}
              onSelect={() => runAction(item.action, item.payload)}
            />
          ))}
        </DropdownMenuGroup>

        <DropdownMenuSeparator />
        <DropdownMenuLabel>Move to</DropdownMenuLabel>
        <DropdownMenuGroup>
          {moveTargets.map((target) => (
            <DropdownMenuItem
              key={target.value}
              inset
              onSelect={() => runAction('move-to', target.value)}
            >
              {target.label}
            </DropdownMenuItem>
          ))}
        </DropdownMenuGroup>

        {filteredExtraActions.length > 0 ? (
          <>
            <DropdownMenuSeparator />
            <DropdownMenuGroup>
              {filteredExtraActions.map((item) => (
                <MessageMenuItem
                  key={item.action}
                  item={item}
                  onSelect={() => runAction(item.action, item.payload)}
                />
              ))}
            </DropdownMenuGroup>
          </>
        ) : null}

        <DropdownMenuSeparator />

        <DropdownMenuGroup>
          {filteredGroups[2].map((item) => (
            <MessageMenuItem
              key={item.action}
              item={item}
              onSelect={() => runAction(item.action, item.payload)}
            />
          ))}
        </DropdownMenuGroup>

        <DropdownMenuSeparator />

        <DropdownMenuGroup>
          {filteredGroups[3].map((item) => (
            <MessageMenuItem
              key={item.action}
              item={item}
              onSelect={() => runAction(item.action, item.payload)}
            />
          ))}
        </DropdownMenuGroup>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

function MessageMenuItem({
  item,
  onSelect,
}: {
  item: ActionItem;
  onSelect: () => void;
}) {
  const Icon = item.icon;

  return (
    <DropdownMenuItem variant={item.variant} onSelect={onSelect}>
      <Icon aria-hidden="true" />
      <span>{item.label}</span>
    </DropdownMenuItem>
  );
}
