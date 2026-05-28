import { useEffect, useRef } from 'react';
import {
  Archive,
  Bookmark,
  Clock,
  Forward,
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
  DropdownMenuSeparator,
} from './ui/dropdown-menu';

export interface MessageActionPopupProps {
  open: boolean;
  onClose: () => void;
  onAction: (action: string, payload?: unknown) => void;
  anchorRect?: DOMRect | null;
  /** Actions to hide based on context (e.g. hide bubble-up in pile views). */
  hiddenActions?: string[];
}

interface ActionItem {
  action: string;
  label: string;
  icon: LucideIcon;
  payload?: unknown;
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
    { action: 'mark-spam', label: 'Mark as spam', icon: X },
    { action: 'trash', label: 'Trash', icon: Trash2 },
  ],
];

const moveTargets = [
  { label: 'Imbox', value: 'imbox' },
  { label: 'Feed', value: 'feed' },
  { label: 'Paper Trail', value: 'papertrail' },
] as const;

export function MessageActionPopup({
  open,
  onClose,
  onAction,
  anchorRect,
  hiddenActions = [],
}: MessageActionPopupProps) {
  const contentRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    if (!open) {
      return undefined;
    }

    function handleKeyDown(event: KeyboardEvent) {
      if (event.key === 'Escape') {
        onClose();
      }
    }

    function handlePointerDown(event: MouseEvent) {
      const target = event.target;
      if (
        target instanceof Node &&
        contentRef.current &&
        !contentRef.current.contains(target)
      ) {
        onClose();
      }
    }

    document.addEventListener('keydown', handleKeyDown);
    document.addEventListener('mousedown', handlePointerDown);

    return () => {
      document.removeEventListener('keydown', handleKeyDown);
      document.removeEventListener('mousedown', handlePointerDown);
    };
  }, [onClose, open]);

  if (!open) {
    return null;
  }

  function runAction(action: string, payload?: unknown) {
    onAction(action, payload);
    onClose();
  }

  const hidden = new Set(hiddenActions);
  const filteredGroups = actionGroups.map((group) =>
    group.filter((item) => !hidden.has(item.action)),
  );

  return (
    <DropdownMenu modal={false} open={open}>
      <DropdownMenuContent
        ref={contentRef}
        align="end"
        aria-label="Message actions"
        className="w-60 border-border-menu bg-bg-surface"
        style={
          anchorRect
            ? {
                position: 'fixed',
                top: anchorRect.bottom + 8,
                left: anchorRect.right - 240,
              }
            : undefined
        }
      >
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

        <DropdownMenuItem onSelect={(event) => event.preventDefault()}>
          Move to
        </DropdownMenuItem>
        <DropdownMenuGroup aria-label="Move targets">
          {moveTargets.map((target) => (
            <button
              key={target.value}
              type="button"
              className="block w-full rounded-md px-6 py-1 text-left text-sm outline-hidden hover:bg-accent hover:text-accent-foreground focus:bg-accent focus:text-accent-foreground"
              onClick={() => runAction('move-to', target.value)}
            >
              {target.label}
            </button>
          ))}
        </DropdownMenuGroup>

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
    <DropdownMenuItem onSelect={onSelect}>
      <Icon aria-hidden="true" />
      <span>{item.label}</span>
    </DropdownMenuItem>
  );
}
