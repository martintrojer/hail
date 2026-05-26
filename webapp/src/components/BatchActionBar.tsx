import { pillButtonClass } from '../lib/buttonStyles';

export interface BatchActionBarProps {
  count: number;
  onDeselectAll: () => void;
  onArchive: () => void;
  onTrash: () => void;
  onSetAside: () => void;
  onReplyLater: () => void;
}

export function BatchActionBar({
  count,
  onDeselectAll,
  onArchive,
  onTrash,
  onSetAside,
  onReplyLater,
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
      <button type="button" onClick={onArchive} className={pillButtonClass('outline')}>
        Archive
      </button>
      <button type="button" onClick={onTrash} className={pillButtonClass('outline')}>
        Trash
      </button>
      <button type="button" onClick={onSetAside} className={pillButtonClass('outline')}>
        Set Aside
      </button>
      <button type="button" onClick={onReplyLater} className={pillButtonClass('outline')}>
        Reply Later
      </button>
    </div>
  );
}
