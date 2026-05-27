import { Button } from './ui/button';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from './ui/dropdown-menu';
import { Separator } from './ui/separator';
import {
  Archive,
  Bookmark,
  Clock,
  Inbox,
  MoreHorizontal,
  Trash2,
  X,
} from './icons';
import type { ListAction } from '../hooks/useListActions';

const actionLabels: Record<ListAction, string> = {
  archive: 'Archive',
  trash: 'Trash',
  'set-aside': 'Set Aside',
  'reply-later': 'Reply Later',
  classify: 'Move to Imbox',
  restore: 'Restore',
  'not-spam': 'Not Spam',
  delete: 'Delete',
  'delete-forever': 'Delete forever',
};

const actionIcons: Partial<Record<ListAction, typeof Archive>> = {
  archive: Archive,
  restore: Archive,
  trash: Trash2,
  delete: Trash2,
  'delete-forever': Trash2,
  'set-aside': Bookmark,
  'reply-later': Clock,
  classify: Inbox,
};

const primaryActions: ListAction[] = ['archive', 'trash', 'set-aside'];
const destructiveActions = new Set<ListAction>(['trash', 'delete', 'delete-forever']);

export interface BatchActionBarProps {
  count: number;
  onDeselectAll: () => void;
  availableActions: ListAction[];
  onAction: (action: ListAction) => void;
}

function ActionButton({
  action,
  onAction,
}: {
  action: ListAction;
  onAction: (action: ListAction) => void;
}) {
  const Icon = actionIcons[action];

  return (
    <Button
      type="button"
      size="sm"
      variant={destructiveActions.has(action) ? 'destructive' : 'outline'}
      onClick={() => onAction(action)}
    >
      {Icon ? <Icon data-icon="inline-start" /> : null}
      {actionLabels[action]}
    </Button>
  );
}

export function BatchActionBar({
  count,
  onDeselectAll,
  availableActions,
  onAction,
}: BatchActionBarProps) {
  const visibleActions = availableActions.filter((action) => primaryActions.includes(action));
  const overflowActions = availableActions.filter((action) => !primaryActions.includes(action));

  return (
    <div className="sticky top-12 z-10 flex flex-wrap items-center gap-2 border-b bg-background/95 px-3 py-2 backdrop-blur supports-backdrop-filter:bg-background/80">
      <span className="text-sm font-medium text-foreground">
        {count} selected
      </span>
      <Separator orientation="vertical" className="hidden min-h-5 sm:block" />
      <Button type="button" variant="ghost" size="sm" onClick={onDeselectAll}>
        <X data-icon="inline-start" />
        Deselect All
      </Button>
      <div className="flex flex-wrap items-center gap-1">
        {visibleActions.map((action) => (
          <ActionButton key={action} action={action} onAction={onAction} />
        ))}
        {overflowActions.length > 0 ? (
          <DropdownMenu>
            <DropdownMenuTrigger asChild>
              <Button type="button" variant="outline" size="sm">
                <MoreHorizontal data-icon="inline-start" />
                More
              </Button>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="start">
              <DropdownMenuGroup>
                {overflowActions.map((action) => {
                  const Icon = actionIcons[action];
                  return (
                    <DropdownMenuItem
                      key={action}
                      variant={destructiveActions.has(action) ? 'destructive' : 'default'}
                      onSelect={() => onAction(action)}
                    >
                      {Icon ? <Icon /> : null}
                      <span>{actionLabels[action]}</span>
                    </DropdownMenuItem>
                  );
                })}
              </DropdownMenuGroup>
            </DropdownMenuContent>
          </DropdownMenu>
        ) : null}
      </div>
    </div>
  );
}
