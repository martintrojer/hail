import { useEffect, useRef, useState, type ReactNode } from 'react';
import { createPortal } from 'react-dom';
import {
  Archive,
  Bookmark,
  ChevronDown,
  Clock,
  Forward,
  Reply,
  StickyNote,
  Trash2,
  X,
  icons,
  iconSizeProps,
  type LucideIcon,
} from './icons';
import { useKeyboardShortcuts } from '../hooks/useKeyboardShortcuts';

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

function popupPosition(anchorRect: DOMRect | null | undefined) {
  if (!anchorRect || typeof window === 'undefined') {
    return { top: 96, left: 24 };
  }

  const width = 240;
  const gutter = 12;
  const top = anchorRect.bottom + window.scrollY + 8;
  const preferredLeft = anchorRect.right + window.scrollX - width;
  const maxLeft = window.scrollX + window.innerWidth - width - gutter;
  const minLeft = window.scrollX + gutter;

  return {
    top,
    left: Math.max(minLeft, Math.min(preferredLeft, maxLeft)),
  };
}

function Divider() {
  return <div className="my-1 border-t border-border-menu" role="separator" />;
}

function MenuButton({
  icon: Icon,
  label,
  children,
  onClick,
}: {
  icon: LucideIcon;
  label: string;
  children?: ReactNode;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      className="flex w-full items-center gap-3 rounded-md px-3 py-2.5 text-left text-[0.875rem] font-medium text-ink-primary hover:bg-bg-hover focus:bg-bg-hover focus:outline-none"
    >
      <Icon {...iconSizeProps.sm} className="shrink-0 text-ink-secondary" aria-hidden="true" />
      <span className="flex-1">{label}</span>
      {children}
    </button>
  );
}

export function MessageActionPopup({
  open,
  onClose,
  onAction,
  anchorRect,
  hiddenActions = [],
}: MessageActionPopupProps) {
  const popupRef = useRef<HTMLDivElement | null>(null);
  const [moveOpen, setMoveOpen] = useState(true);

  useKeyboardShortcuts(
    { onEscape: onClose },
    { enabled: open },
  );

  useEffect(() => {
    if (!open) {
      return undefined;
    }

    function handlePointerDown(event: MouseEvent) {
      const target = event.target;
      if (
        target instanceof Node &&
        popupRef.current &&
        !popupRef.current.contains(target)
      ) {
        onClose();
      }
    }

    document.addEventListener('mousedown', handlePointerDown);

    return () => {
      document.removeEventListener('mousedown', handlePointerDown);
    };
  }, [onClose, open]);

  if (!open || typeof document === 'undefined') {
    return null;
  }

  const position = popupPosition(anchorRect);

  function runAction(action: string, payload?: unknown) {
    onAction(action, payload);
    onClose();
  }

  const hidden = new Set(hiddenActions);
  const filteredGroups = actionGroups.map((group) =>
    group.filter((item) => !hidden.has(item.action)),
  );

  const popup = (
    <div
      ref={popupRef}
      role="menu"
      aria-label="Message actions"
      className="absolute z-50 w-60 rounded-lg border border-border-menu bg-bg-surface p-1.5 shadow-md"
      style={{ top: position.top, left: position.left }}
    >
      {filteredGroups[0].map((item) => (
        <MenuButton
          key={item.action}
          icon={item.icon}
          label={item.label}
          onClick={() => runAction(item.action, item.payload)}
        />
      ))}

      <Divider />

      {filteredGroups[1].map((item) => (
        <MenuButton
          key={item.action}
          icon={item.icon}
          label={item.label}
          onClick={() => runAction(item.action, item.payload)}
        />
      ))}

      <MenuButton
        icon={ChevronDown}
        label="Move to"
        onClick={() => setMoveOpen((current) => !current)}
      >
        <ChevronDown
          {...iconSizeProps.sm}
          aria-hidden="true"
          className={`text-ink-secondary transition-transform ${moveOpen ? 'rotate-180' : ''}`}
        />
      </MenuButton>
      {moveOpen ? (
        <div className="mb-1 ml-7 border-l border-border-hairline pl-2" role="group" aria-label="Move targets">
          {moveTargets.map((target) => (
            <button
              key={target.value}
              type="button"
              className="block w-full rounded-md px-3 py-2 text-left text-[0.875rem] font-medium text-ink-primary hover:bg-bg-hover focus:bg-bg-hover focus:outline-none"
              onClick={() => runAction('move-to', target.value)}
            >
              {target.label}
            </button>
          ))}
        </div>
      ) : null}

      <Divider />

      {filteredGroups[2].map((item) => (
        <MenuButton
          key={item.action}
          icon={item.icon}
          label={item.label}
          onClick={() => runAction(item.action, item.payload)}
        />
      ))}

      <Divider />

      {filteredGroups[3].map((item) => (
        <MenuButton
          key={item.action}
          icon={item.icon}
          label={item.label}
          onClick={() => runAction(item.action, item.payload)}
        />
      ))}
    </div>
  );

  return createPortal(popup, document.body);
}
