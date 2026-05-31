import { useMemo, useState } from 'react';
import type { ThreadAttachment } from '../api/client';
import { formatBytes } from '../lib/format';
import { Paperclip } from './icons';
import { Button } from './ui/button';
import { Card, CardContent } from './ui/card';

const COLLAPSED_ATTACHMENT_COUNT = 5;

interface AttachmentListProps {
  items: ThreadAttachment[];
}

export function AttachmentList({ items }: AttachmentListProps) {
  const [expanded, setExpanded] = useState(false);
  const attachments = useMemo(
    () => items.filter((item) => !item.inline),
    [items],
  );

  if (attachments.length === 0) {
    return null;
  }

  const hiddenCount = Math.max(
    0,
    attachments.length - COLLAPSED_ATTACHMENT_COUNT,
  );
  const visible = expanded
    ? attachments
    : attachments.slice(0, COLLAPSED_ATTACHMENT_COUNT);

  return (
    <Card size="sm" className="mt-5 bg-muted/20">
      <CardContent className="p-3">
        <div className="flex flex-col gap-2" aria-label="Attachments">
          {visible.map((attachment) => (
            <div
              key={`${attachment.blob_id}-${attachment.filename}`}
              className="flex min-w-0 items-center gap-3 rounded-md border bg-background px-3 py-2"
            >
              <Paperclip
                aria-hidden="true"
                className="size-4 shrink-0 text-muted-foreground"
              />
              <div className="min-w-0 flex-1">
                <p className="truncate text-sm font-medium text-foreground">
                  {attachment.filename || 'Attachment'}
                </p>
                <p className="text-xs text-muted-foreground">
                  {formatBytes(attachment.size)} · {attachment.mime_type}
                </p>
              </div>
              <Button asChild variant="outline" size="sm">
                <a
                  href={attachment.download_url}
                  download={attachment.filename || undefined}
                >
                  Download
                </a>
              </Button>
            </div>
          ))}
        </div>

        {!expanded && hiddenCount > 0 ? (
          <Button
            type="button"
            variant="ghost"
            size="sm"
            className="mt-2"
            onClick={() => setExpanded(true)}
          >
            Show {hiddenCount} more
          </Button>
        ) : null}
      </CardContent>
    </Card>
  );
}
