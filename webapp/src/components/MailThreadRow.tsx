import { cn } from '../lib/utils';
import { type MailViewItem } from '../api/client';
import { MailRow } from './MailRow';
import { ThreadLink } from './ThreadLink';

interface MailThreadRowProps {
  item: MailViewItem;
  selected?: boolean;
  onToggleSelect?: () => void;
  className?: string;
}

export function MailThreadRow({
  item,
  selected,
  onToggleSelect,
  className,
}: MailThreadRowProps) {
  return (
    <ThreadLink
      threadId={item.thread_id}
      mailListItem
      className={cn(
        'block border-b border-l-2 border-b-border border-l-transparent py-1 focus-visible:border-l-primary focus-visible:bg-accent focus-visible:outline-none hover:bg-muted/60',
        selected && 'bg-accent',
        className,
      )}
      ariaLabel={`Open ${item.subject || 'thread'} from ${item.from || 'unknown sender'}`}
    >
      <MailRow
        from={item.from || 'Unknown sender'}
        subject={item.subject || '(no subject)'}
        preview={item.preview || 'No preview available.'}
        receivedAt={item.received_at}
        unread={item.unread}
        hasNotes={item.has_notes}
        selected={selected}
        onToggleSelect={onToggleSelect}
        labels={item.labels}
        messageCount={item.message_count}
        unreadCount={item.unread_count}
      />
    </ThreadLink>
  );
}
