import { pillButtonClass } from '../lib/buttonStyles';
import type { ListAction } from '../hooks/useListActions';

const actionLabels: Record<ListAction, string> = {
  archive: 'Archive',
  trash: 'Trash',
  'set-aside': 'Set Aside',
  'reply-later': 'Reply Later',
  classify: 'Move to Imbox',
  restore: 'Restore',
  delete: 'Delete',
  'delete-forever': 'Delete forever',
};

const actionVariants: Partial<Record<ListAction, Parameters<typeof pillButtonClass>[0]>> = {
  'delete-forever': 'danger',
  delete: 'danger',
};

export interface BatchActionBarProps {
  count: number;
  onDeselectAll: () => void;
  availableActions: ListAction[];
  onAction: (action: ListAction) => void;
}

export function BatchActionBar({
  count,
  onDeselectAll,
  availableActions,
  onAction,
}: BatchActionBarProps) {
  return (
    <div className="sticky top-0 z-10 flex flex-wrap items-center gap-2 border-b border-border-hairline bg-bg-surface px-3 py-3">
      <span className="mr-2 text-sm font-semibold text-ink-primary">
        {count} selected
      </span>
      <button
        type="button"
        onClick={onDeselectAll}
        className="rounded-full px-3 py-1.5 text-sm font-semibold text-ink-secondary focus-ring outline-none hover:bg-bg-hover hover:text-ink-primary"
      >
        Deselect All
      </button>
      {availableActions.map((action) => (
        <button
          key={action}
          type="button"
          onClick={() => onAction(action)}
          className={pillButtonClass(actionVariants[action] ?? 'outline')}
        >
          {actionLabels[action]}
        </button>
      ))}
    </div>
  );
}
