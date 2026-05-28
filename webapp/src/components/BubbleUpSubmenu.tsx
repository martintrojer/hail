import { useEffect, useRef } from 'react';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuItem,
} from './ui/dropdown-menu';

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

export function BubbleUpSubmenu({
  open,
  onClose,
  onSelect,
  anchorRect,
}: BubbleUpSubmenuProps) {
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

  function selectOption(option: string) {
    onSelect(option);
    onClose();
  }

  return (
    <DropdownMenu modal={false} open={open}>
      <DropdownMenuContent
        ref={contentRef}
        aria-label="Bubble up time options"
        className="w-[220px] border-border-menu bg-bg-surface"
        style={
          anchorRect
            ? {
                position: 'fixed',
                top: anchorRect.top,
                left: anchorRect.right + 8,
              }
            : undefined
        }
      >
        <DropdownMenuGroup>
          {bubbleUpOptions.map((option) => (
            <DropdownMenuItem key={option} onSelect={() => selectOption(option)}>
              {option}
            </DropdownMenuItem>
          ))}
        </DropdownMenuGroup>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
