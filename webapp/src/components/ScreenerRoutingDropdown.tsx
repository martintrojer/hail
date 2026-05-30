import type { ReactNode } from 'react';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuTrigger,
} from './ui/dropdown-menu';

export type ScreenerRoutingDestination = 'imbox' | 'feed' | 'papertrail';

export const routingDestinationLabels = {
  imbox: 'The Imbox',
  feed: 'The Feed',
  papertrail: 'Paper Trail',
} as const satisfies Record<ScreenerRoutingDestination, string>;

export interface ScreenerRoutingDropdownProps {
  onSelect: (destination: ScreenerRoutingDestination) => void;
  children: ReactNode;
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
  onSelect,
  children,
  value = 'imbox',
}: ScreenerRoutingDropdownProps) {
  return (
    <DropdownMenu modal={false}>
      <DropdownMenuTrigger asChild>{children}</DropdownMenuTrigger>
      <DropdownMenuContent
        aria-label="Screener routing destinations"
        className="w-[180px]"
        side="bottom"
        align="start"
        sideOffset={8}
      >
        <DropdownMenuRadioGroup value={value}>
          {routingOptions.map((option) => (
            <DropdownMenuRadioItem
              key={option.value}
              value={option.value}
              onSelect={() => onSelect(option.value)}
            >
              {option.label}
            </DropdownMenuRadioItem>
          ))}
        </DropdownMenuRadioGroup>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
