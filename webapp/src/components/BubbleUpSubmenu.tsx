import type { ReactNode } from 'react';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuItem,
  DropdownMenuSub,
  DropdownMenuSubContent,
  DropdownMenuSubTrigger,
} from './ui/dropdown-menu';

export interface BubbleUpSubmenuProps {
  open: boolean;
  onClose: () => void;
  onSelect: (option: string) => void;
}

export const bubbleUpOptions = [
  'Later today',
  'Tomorrow morning',
  'This weekend',
  'Next week',
  'Pick a date…',
] as const;

export function BubbleUpDropdownSub({
  children,
  onSelect,
}: {
  children: ReactNode;
  onSelect: (option: string) => void;
}) {
  return (
    <DropdownMenuSub>
      <DropdownMenuSubTrigger>{children}</DropdownMenuSubTrigger>
      <DropdownMenuSubContent aria-label="Bubble up time options" className="w-56">
        <DropdownMenuGroup>
          {bubbleUpOptions.map((option) => (
            <DropdownMenuItem key={option} onSelect={() => onSelect(option)}>
              {option}
            </DropdownMenuItem>
          ))}
        </DropdownMenuGroup>
      </DropdownMenuSubContent>
    </DropdownMenuSub>
  );
}

export function BubbleUpSubmenu({
  open,
  onClose,
  onSelect,
}: BubbleUpSubmenuProps) {
  function selectOption(option: string) {
    onSelect(option);
  }

  return (
    <DropdownMenu
      modal={false}
      open={open}
      onOpenChange={(nextOpen) => {
        if (!nextOpen) {
          onClose();
        }
      }}
    >
      <DropdownMenuContent aria-label="Bubble up time options" className="w-56">
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
