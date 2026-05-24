import { useEffect, useRef } from 'react';
import { createPortal } from 'react-dom';

export type ScreenerRoutingDestination = 'imbox' | 'feed' | 'papertrail';

export interface ScreenerRoutingDropdownProps {
  open: boolean;
  onClose: () => void;
  onSelect: (destination: ScreenerRoutingDestination) => void;
  anchorRect?: DOMRect | null;
}

const routingOptions: Array<{
  value: ScreenerRoutingDestination;
  label: string;
}> = [
  { value: 'imbox', label: 'The Imbox' },
  { value: 'feed', label: 'The Feed' },
  { value: 'papertrail', label: 'Paper Trail' },
];

function dropdownPosition(anchorRect: DOMRect | null | undefined) {
  if (!anchorRect || typeof window === 'undefined') {
    return { top: 96, left: 24 };
  }

  const width = 180;
  const gutter = 12;
  const top = anchorRect.bottom + window.scrollY + 8;
  const preferredLeft = anchorRect.left + window.scrollX;
  const maxLeft = window.scrollX + window.innerWidth - width - gutter;
  const minLeft = window.scrollX + gutter;

  return {
    top,
    left: Math.max(minLeft, Math.min(preferredLeft, maxLeft)),
  };
}

export function ScreenerRoutingDropdown({
  open,
  onClose,
  onSelect,
  anchorRect,
}: ScreenerRoutingDropdownProps) {
  const dropdownRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    if (!open) {
      return undefined;
    }

    function handlePointerDown(event: MouseEvent) {
      const target = event.target;
      if (
        target instanceof Node &&
        dropdownRef.current &&
        !dropdownRef.current.contains(target)
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

  const position = dropdownPosition(anchorRect);

  function selectDestination(destination: ScreenerRoutingDestination) {
    onSelect(destination);
    onClose();
  }

  const dropdown = (
    <div
      ref={dropdownRef}
      role="menu"
      aria-label="Screener routing destinations"
      className="absolute z-50 w-[180px] rounded-lg border border-border-menu bg-bg-surface p-1.5 shadow-md"
      style={{ top: position.top, left: position.left }}
    >
      {routingOptions.map((option) => {
        const isDefault = option.value === 'imbox';

        return (
          <button
            key={option.value}
            type="button"
            role="menuitem"
            className={`flex w-full items-center gap-2 rounded-md px-3 py-2 text-left text-sm font-medium text-ink-primary hover:bg-bg-hover focus:bg-bg-hover focus:outline-none ${
              isDefault ? 'bg-bg-selected' : ''
            }`}
            onClick={() => selectDestination(option.value)}
          >
            <span className="flex-1">{option.label}</span>
            {isDefault ? (
              <span aria-hidden="true" className="text-ink-secondary">
                ✓
              </span>
            ) : null}
          </button>
        );
      })}
    </div>
  );

  return createPortal(dropdown, document.body);
}
