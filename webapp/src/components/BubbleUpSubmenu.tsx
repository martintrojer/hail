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
