import type { ReactNode } from 'react';
import { Check } from './icons';
import { cn } from '../lib/utils';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuItem,
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
        <DropdownMenuGroup>
          {routingOptions.map((option) => {
            const isSelected = option.value === value;

            return (
              <DropdownMenuItem
                key={option.value}
                aria-checked={isSelected}
                onSelect={() => onSelect(option.value)}
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
