import { useEffect, useRef } from 'react';
import { createPortal } from 'react-dom';

export interface BubbleUpSubmenuProps {
  open: boolean;
  onClose: () => void;
  onSelect: (option: string) => void;
  anchorRect?: DOMRect | null;
}

const bubbleUpOptions = [
  'Later today',
  'Tomorrow morning',
  'This weekend',
  'Next week',
  'Pick a date…',
] as const;

function submenuPosition(anchorRect: DOMRect | null | undefined) {
  if (!anchorRect || typeof window === 'undefined') {
    return { top: 96, left: 24 };
  }

  const width = 220;
  const gutter = 12;
  const top = anchorRect.top + window.scrollY;
  const preferredLeft = anchorRect.right + window.scrollX + 8;
  const maxLeft = window.scrollX + window.innerWidth - width - gutter;
  const minLeft = window.scrollX + gutter;

  return {
    top,
    left: Math.max(minLeft, Math.min(preferredLeft, maxLeft)),
  };
}

export function BubbleUpSubmenu({
  open,
  onClose,
  onSelect,
  anchorRect,
}: BubbleUpSubmenuProps) {
  const submenuRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    if (!open) {
      return undefined;
    }

    function handlePointerDown(event: MouseEvent) {
      const target = event.target;
      if (
        target instanceof Node &&
        submenuRef.current &&
        !submenuRef.current.contains(target)
      ) {
        onClose();
      }
    }

    function handleKeyDown(event: KeyboardEvent) {
      if (event.key === 'Escape') {
        onClose();
      }
    }

    document.addEventListener('mousedown', handlePointerDown);
    document.addEventListener('keydown', handleKeyDown);

    return () => {
      document.removeEventListener('mousedown', handlePointerDown);
      document.removeEventListener('keydown', handleKeyDown);
    };
  }, [onClose, open]);

  if (!open || typeof document === 'undefined') {
    return null;
  }

  const position = submenuPosition(anchorRect);

  function selectOption(option: string) {
    onSelect(option);
    onClose();
  }

  const submenu = (
    <div
      ref={submenuRef}
      role="menu"
      aria-label="Bubble up time options"
      className="absolute z-50 w-[220px] rounded-lg border border-border-menu bg-bg-surface p-1.5 shadow-md"
      style={{ top: position.top, left: position.left }}
    >
      {bubbleUpOptions.map((option) => (
        <button
          key={option}
          type="button"
          role="menuitem"
          className="block w-full rounded-md px-3 py-2.5 text-left text-sm font-medium text-ink-primary hover:bg-bg-hover focus:bg-bg-hover focus:outline-none"
          onClick={() => selectOption(option)}
        >
          {option}
        </button>
      ))}
    </div>
  );

  return createPortal(submenu, document.body);
}
