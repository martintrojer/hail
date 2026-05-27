import type { MouseEvent, ReactNode } from 'react';
import { formatDateTime } from '../lib/dates';
import { previewClass, senderNameClass, subjectClass, timeClass } from '../lib/mailRowStyles';
import { Badge } from './ui/badge';
import { Checkbox } from './ui/checkbox';
import { StickyNote } from './icons';

export interface MailRowProps {
  from: string;
  subject: string;
  preview: string;
  receivedAt: string | null | undefined;
  receivedAtFallback?: string;
  unread?: boolean;
  hasNotes?: boolean;
  selected?: boolean;
  onToggleSelect?: () => void;
  children?: ReactNode;
}

function NewPill() {
  return (
    <Badge variant="secondary" className="h-4 px-1.5 text-[0.65rem] uppercase tracking-wide">
      New
    </Badge>
  );
}

function SelectionToggle({
  from,
  selected,
  onToggleSelect,
}: {
  from: string;
  selected: boolean;
  onToggleSelect: () => void;
}) {
  function handleClick(event: MouseEvent<HTMLButtonElement>) {
    event.preventDefault();
    event.stopPropagation();
    onToggleSelect();
  }

  return (
    <Checkbox
      checked={selected}
      onClick={handleClick}
      aria-label={`${selected ? 'Deselect' : 'Select'} ${from || 'Unknown sender'}`}
      className="mt-0.5 shrink-0"
    />
  );
}

export function MailRow({
  from,
  subject,
  preview,
  receivedAt,
  receivedAtFallback,
  unread = false,
  hasNotes = false,
  selected = false,
  onToggleSelect,
  children,
}: MailRowProps) {
  const content = (
    <div className="flex min-w-0 flex-1 items-start gap-2 px-3 py-2">
      {onToggleSelect ? (
        <SelectionToggle from={from} selected={selected} onToggleSelect={onToggleSelect} />
      ) : null}
      <div className="min-w-0 flex-1">
        <div className="flex items-baseline justify-between gap-4">
          <div className="flex min-w-0 items-center gap-2">
            <p className={`${senderNameClass} ${unread ? 'font-bold' : ''}`}>
              {from || 'Unknown sender'}
            </p>
            {unread ? <NewPill /> : null}
          </div>
          <time className={timeClass}>{formatDateTime(receivedAt, receivedAtFallback)}</time>
        </div>
        <p className={`mt-1 flex items-center ${subjectClass}`}>
          <span className="truncate">{subject || '(no subject)'}</span>
          {hasNotes ? (
            <StickyNote
              className="ml-1.5 inline-block shrink-0 align-[-0.125em] text-muted-foreground"
              aria-label="Thread has notes"
            />
          ) : null}
        </p>
        {preview ? <p className={`mt-1 ${previewClass}`}>{preview}</p> : null}
      </div>
    </div>
  );

  if (!children) {
    return content;
  }

  return (
    <div className="flex items-start justify-between gap-3">
      {content}
      <div className="flex shrink-0 flex-col items-end gap-2 sm:flex-row sm:items-center">
        {children}
      </div>
    </div>
  );
}
