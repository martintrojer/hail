import { useState, type KeyboardEvent, type MouseEvent, type ReactNode } from 'react';
import { cn } from '../lib/utils';
import { formatDateTime } from '../lib/dates';
import { actionErrorMessage } from '../lib/errorMessages';
import { useApiClient } from '../api/ApiClientProvider';
import { useListActions } from '../hooks/useListActions';
import { previewClass, senderNameClass, subjectClass, timeClass } from '../lib/mailRowStyles';
import { Badge } from './ui/badge';
import { Checkbox } from './ui/checkbox';
import { MoreHorizontal, StickyNote } from './icons';
import { LabelChips } from './LabelChips';
import type { HailApiClient } from '../api/client';
import type { components } from '../api/types';
import { Button } from './ui/button';
import { MessageActionPopup, markReadAction, markUnreadAction } from './MessageActionPopup';

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
  labels?: components['schemas']['LabelResponse'][];
  children?: ReactNode;
}


export function MailRowQuickActionsMenu({
  threadId,
  subject,
  unread,
  selected,
  client,
}: {
  threadId?: string;
  subject: string;
  unread: boolean;
  selected: boolean;
  client?: HailApiClient;
}) {
  const [open, setOpen] = useState(false);
  const listActions = useListActions({
    client,
    availableActions: ['archive', 'trash', 'set-aside', 'reply-later', 'classify'],
  });
  const apiClient = useApiClient();

  if (!threadId || selected) {
    return null;
  }

  function stopKeyEvent(event: KeyboardEvent<HTMLButtonElement>) {
    if (event.key === 'Enter' || event.key === ' ') {
      event.stopPropagation();
    }
  }

  async function handleAction(action: string, payload?: unknown) {
    setOpen(false);
    if (!threadId) {
      return;
    }

    if (action === 'move-to') {
      if (payload !== 'imbox' && payload !== 'feed' && payload !== 'papertrail') {
        return;
      }
      await listActions.run('classify', threadId, { classifyTo: payload });
      return;
    }

    if (action === 'archive' || action === 'trash') {
      await listActions.run(action, threadId);
      return;
    }

    if (action === 'set-aside') {
      await listActions.setAside(threadId);
      return;
    }

    if (action === 'reply-later') {
      await listActions.replyLater(threadId);
      return;
    }

    if (action === 'mark-read') {
      await apiClient.markThread(threadId, true);
      return;
    }

    if (action === 'mark-unread') {
      await apiClient.markThread(threadId, false);
    }
  }

  return (
    <div
      data-hail-row-actions="true"
      className={cn(
        'ml-1 flex shrink-0 items-center opacity-90 transition-opacity sm:opacity-0 sm:group-hover/mail-row:opacity-100 sm:group-focus-within/mail-row:opacity-100',
        open && 'sm:opacity-100',
      )}
    >
      <MessageActionPopup
        open={open}
        onClose={() => setOpen(false)}
        onOpenChange={setOpen}
        trigger={
          <Button
            type="button"
            variant="ghost"
            size="icon-xs"
            aria-label={`Actions for ${subject || '(no subject)'}`}
            aria-haspopup="menu"
            aria-expanded={open}
            disabled={listActions.isBusy}
            onKeyDown={stopKeyEvent}
            onMouseDown={(event) => event.stopPropagation()}
            onClick={(event) => {
              event.preventDefault();
              event.stopPropagation();
              setOpen((value) => !value);
            }}
          >
            <MoreHorizontal aria-hidden="true" />
          </Button>
        }
        onAction={(action, payload) => {
          void handleAction(action, payload);
        }}
        hiddenActions={[
          'reply',
          'reply-all',
          'forward',
          'bubble-up',
          'add-note',
          'mark-spam',
        ]}
        extraActions={[unread ? markReadAction : markUnreadAction]}
      />
      {listActions.error ? (
        <span role="alert" className="sr-only">
          {actionErrorMessage(listActions.error, 'Thread action')}
        </span>
      ) : null}
    </div>
  );
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
  labels = [],
  children,
}: MailRowProps) {
  const content = (
    <div className="group/mail-row flex min-w-0 flex-1 items-start gap-2 px-3 py-2">
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
        <LabelChips labels={labels} className="mt-1.5 flex min-w-0 flex-wrap items-center gap-1" />
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
