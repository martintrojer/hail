import { useEffect, useRef } from 'react';
import { Check } from './icons';
import { cn } from '../lib/utils';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuItem,
} from './ui/dropdown-menu';

export type ScreenerRoutingDestination = 'imbox' | 'feed' | 'papertrail';

export const routingDestinationLabels = {
  imbox: 'The Imbox',
  feed: 'The Feed',
  papertrail: 'Paper Trail',
} as const satisfies Record<ScreenerRoutingDestination, string>;

export interface ScreenerRoutingDropdownProps {
  open: boolean;
  onClose: () => void;
  onSelect: (destination: ScreenerRoutingDestination) => void;
  anchorRect?: DOMRect | null;
  value?: ScreenerRoutingDestination;
}

const routingOptions: Array<{
  value: ScreenerRoutingDestination;
  label: string;
}> = [
  { value: 'imbox', label: routingDestinationLabels.imbox },
  { value: 'feed', label: routingDestinationLabels.feed },
  { value: 'papertrail', label: routingDestinationLabels.papertrail },
];

export function ScreenerRoutingDropdown({
  open,
  onClose,
  onSelect,
  anchorRect,
  value = 'imbox',
}: ScreenerRoutingDropdownProps) {
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

  function selectDestination(destination: ScreenerRoutingDestination) {
    onSelect(destination);
    onClose();
  }

  return (
    <DropdownMenu modal={false} open={open}>
      <DropdownMenuContent
        ref={contentRef}
        aria-label="Screener routing destinations"
        className="w-[180px]"
        style={
          anchorRect
            ? {
                position: 'fixed',
                top: anchorRect.bottom + 8,
                left: anchorRect.left,
              }
            : undefined
        }
      >
        <DropdownMenuGroup>
          {routingOptions.map((option) => {
            const isSelected = option.value === value;

            return (
              <DropdownMenuItem
                key={option.value}
                onSelect={() => selectDestination(option.value)}
                className={cn(isSelected && 'bg-muted')}
              >
                <span className="flex-1">{option.label}</span>
                {isSelected ? <Check aria-hidden="true" /> : null}
              </DropdownMenuItem>
            );
          })}
        </DropdownMenuGroup>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
