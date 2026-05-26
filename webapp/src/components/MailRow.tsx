import type { MouseEvent, ReactNode } from 'react';
import { formatDateTime } from '../lib/dates';
import { previewClass, senderNameClass, subjectClass, timeClass } from '../lib/mailRowStyles';
import { Check, StickyNote, iconSizeProps } from './icons';

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
    <span className="shrink-0 rounded-full bg-accent-yellow px-2 py-0.5 text-[0.7rem] font-semibold uppercase leading-tight tracking-wider text-ink-primary">
      New
    </span>
  );
}

function senderInitial(from: string) {
  return (from.trim()[0] ?? '?').toUpperCase();
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
    <button
      type="button"
      onClick={handleClick}
      aria-pressed={selected}
      aria-label={`${selected ? 'Deselect' : 'Select'} ${from || 'Unknown sender'}`}
      className={`grid h-9 w-9 shrink-0 place-items-center rounded-full border text-sm font-semibold transition focus-ring outline-none ${
        selected
          ? 'border-accent-blue bg-accent-blue text-white'
          : 'border-border-menu bg-bg-hover text-ink-secondary hover:border-accent-blue hover:text-accent-blue'
      }`}
    >
      {selected ? <Check {...iconSizeProps.sm} aria-hidden="true" /> : senderInitial(from)}
    </button>
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
    <div className="flex min-w-0 flex-1 items-start gap-3">
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
              {...iconSizeProps.sm}
              className="ml-1.5 inline-block shrink-0 align-[-0.125em] text-ink-tertiary"
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
    <div className="flex items-start justify-between gap-4">
      {content}
      <div className="flex shrink-0 flex-col items-end gap-2 sm:flex-row sm:items-center">
        {children}
      </div>
    </div>
  );
}
