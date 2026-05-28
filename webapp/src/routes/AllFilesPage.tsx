import { Link } from '@tanstack/react-router';
import type { AttachmentItem } from '../api/client';
import { useAttachments } from '../api/query';
import { ErrorState } from '../components/ErrorState';
import { LoadingState } from '../components/LoadingState';
import { StateCard } from '../components/StateCard';
import { Paperclip } from '../components/icons';
import { Badge } from '../components/ui/badge';
import { Button } from '../components/ui/button';
import {
  Card,
  CardAction,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from '../components/ui/card';
import { AppShell } from '../layout/AppShell';
import { formatDateTime } from '../lib/dates';
import { viewErrorMessage } from '../lib/errorMessages';

interface AllFilesPageProps {
  client?: Parameters<typeof useAttachments>[0];
}

function fileSize(bytes: number) {
  if (!Number.isFinite(bytes) || bytes <= 0) {
    return 'Unknown size';
  }

  const units = ['B', 'KB', 'MB', 'GB'];
  let value = bytes;
  let unitIndex = 0;
  while (value >= 1024 && unitIndex < units.length - 1) {
    value /= 1024;
    unitIndex += 1;
  }

  const digits = value >= 10 || unitIndex === 0 ? 0 : 1;
  return `${value.toFixed(digits)} ${units[unitIndex]}`;
}

function fileKind(type: string) {
  const [major, subtype] = type.split('/');
  if (!major) {
    return 'File';
  }
  if (major === 'application' && subtype) {
    return subtype.toUpperCase();
  }
  return major.charAt(0).toUpperCase() + major.slice(1);
}

function SummaryCard({ label, value }: { label: string; value: string | number }) {
  return (
    <Card size="sm">
      <CardHeader>
        <CardDescription className="text-xs font-semibold uppercase tracking-[0.2em]">
          {label}
        </CardDescription>
        <CardTitle className="text-2xl">{value}</CardTitle>
      </CardHeader>
    </Card>
  );
}

function AllFilesSummary({ items }: { items: AttachmentItem[] }) {
  const totalBytes = items.reduce((sum, item) => sum + (item.size || 0), 0);
  const threadCount = new Set(items.map((item) => item.context.thread_id)).size;

  return (
    <div className="grid gap-3 sm:grid-cols-3">
      <SummaryCard label="Files" value={items.length} />
      <SummaryCard label="Threads" value={threadCount} />
      <SummaryCard label="Storage shown" value={fileSize(totalBytes)} />
    </div>
  );
}

function AttachmentRow({ item }: { item: AttachmentItem }) {
  return (
    <Card size="sm" className="transition hover:bg-muted/50">
      <CardHeader>
        <div className="flex min-w-0 items-start gap-4">
          <div className="grid size-10 shrink-0 place-items-center rounded-lg bg-muted text-primary">
            <Paperclip aria-hidden="true" strokeWidth={1.5} />
          </div>
          <div className="min-w-0 flex-1">
            <CardTitle className="truncate">{item.name || 'Attachment'}</CardTitle>
            <CardDescription className="mt-1">
              <Badge variant="outline">{fileKind(item.type)}</Badge>{' '}
              <span aria-label={`Attachment size ${fileSize(item.size)}`}>File size</span>
            </CardDescription>
          </div>
        </div>
        <CardAction className="flex gap-2">
          <Button asChild size="sm">
            <a href={item.download_url} target="_blank" rel="noreferrer">
              Open
            </a>
          </Button>
          <Button asChild variant="outline" size="sm">
            <a href={item.download_url} download={item.name || undefined}>
              Download
            </a>
          </Button>
        </CardAction>
      </CardHeader>

      <CardContent>
        <div className="rounded-lg border bg-muted/30 p-3">
          <div className="flex flex-col gap-2 sm:flex-row sm:items-center sm:justify-between">
            <Link
              to="/thread/$threadId"
              params={{ threadId: item.context.thread_id }}
              search={{ from: undefined }}
              className="min-w-0 truncate font-medium text-foreground outline-none hover:text-primary focus-visible:ring-3 focus-visible:ring-ring/50"
            >
              {item.context.subject || '(no subject)'}
            </Link>
            <time className="shrink-0 text-xs text-muted-foreground">
              {formatDateTime(item.context.received_at)}
            </time>
          </div>
          <p className="mt-1 truncate text-sm text-muted-foreground">
            {item.context.from || 'Unknown sender'}
          </p>
          <p className="mt-2 line-clamp-2 text-sm leading-6 text-muted-foreground">
            {item.context.preview || 'No message preview available.'}
          </p>
        </div>
      </CardContent>
    </Card>
  );
}

function AllFilesList({ items }: { items: AttachmentItem[] }) {
  if (items.length === 0) {
    return (
      <StateCard
        title="No files yet"
        body="Attachments from your recent mail will appear here with links back to their threads."
      />
    );
  }

  return (
    <div className="flex flex-col gap-3">
      {items.map((item) => (
        <AttachmentRow
          key={`${item.context.email_id}:${item.blob_id}:${item.name}`}
          item={item}
        />
      ))}
    </div>
  );
}

export function AllFilesPage({ client }: AllFilesPageProps) {
  const query = useAttachments(client);

  let list;
  if (query.isPending) {
    list = <LoadingState label="Loading files" />;
  } else if (query.isError) {
    list = (
      <ErrorState
        message={viewErrorMessage(query.error, 'All Files')}
      />
    );
  } else {
    list = (
      <>
        <AllFilesSummary items={query.data.items} />
        <AllFilesList items={query.data.items} />
      </>
    );
  }

  return (
    <AppShell
      title="All Files"
      description="Every recent attachment in one place, with the mail thread it came from."
      list={<div className="flex flex-col gap-5">{list}</div>}
      wide
    />
  );
}
