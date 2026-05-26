import { Link } from '@tanstack/react-router';
import type { AttachmentItem } from '../api/client';
import { useAttachments } from '../api/query';
import { ErrorState } from '../components/ErrorState';
import { LoadingState } from '../components/LoadingState';
import { StateCard } from '../components/StateCard';
import { Paperclip } from '../components/icons';
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

function AllFilesSummary({ items }: { items: AttachmentItem[] }) {
  const totalBytes = items.reduce((sum, item) => sum + (item.size || 0), 0);
  const threadCount = new Set(items.map((item) => item.context.thread_id)).size;

  return (
    <div className="grid gap-3 sm:grid-cols-3">
      <div className="rounded-2xl bg-bg-surface p-4 shadow-sm shadow-ink-primary/5">
        <p className="text-xs font-semibold uppercase tracking-[0.2em] text-ink-tertiary">Files</p>
        <p className="mt-2 text-2xl font-semibold text-ink-primary">{items.length}</p>
      </div>
      <div className="rounded-2xl bg-bg-surface p-4 shadow-sm shadow-ink-primary/5">
        <p className="text-xs font-semibold uppercase tracking-[0.2em] text-ink-tertiary">Threads</p>
        <p className="mt-2 text-2xl font-semibold text-ink-primary">{threadCount}</p>
      </div>
      <div className="rounded-2xl bg-bg-surface p-4 shadow-sm shadow-ink-primary/5">
        <p className="text-xs font-semibold uppercase tracking-[0.2em] text-ink-tertiary">Storage shown</p>
        <p className="mt-2 text-2xl font-semibold text-ink-primary">{fileSize(totalBytes)}</p>
      </div>
    </div>
  );
}

function AttachmentRow({ item }: { item: AttachmentItem }) {
  return (
    <article className="group rounded-2xl bg-bg-surface p-4 shadow-sm shadow-ink-primary/5 transition hover:bg-bg-hover">
      <div className="flex items-start gap-4">
        <div className="grid h-12 w-12 shrink-0 place-items-center rounded-2xl bg-bg-selected text-accent-blue">
          <Paperclip aria-hidden="true" size={22} strokeWidth={1.5} />
        </div>
        <div className="min-w-0 flex-1">
          <div className="flex flex-col gap-3 sm:flex-row sm:items-start sm:justify-between">
            <div className="min-w-0">
              <h2 className="truncate text-lg font-semibold text-ink-primary">{item.name || 'Attachment'}</h2>
              <p className="mt-1 text-sm text-ink-secondary">
                {fileKind(item.type)} · {fileSize(item.size)}
              </p>
            </div>
            <div className="flex shrink-0 gap-2">
              <a
                href={item.download_url}
                target="_blank"
                rel="noreferrer"
                className="rounded-full bg-accent-blue px-3 py-1.5 text-sm font-semibold text-white focus-ring outline-none hover:bg-accent-blue-hover"
              >
                Open
              </a>
              <a
                href={item.download_url}
                download={item.name || undefined}
                className="rounded-full border border-border-menu px-3 py-1.5 text-sm font-semibold text-ink-secondary focus-ring outline-none hover:bg-bg-page hover:text-ink-primary"
              >
                Download
              </a>
            </div>
          </div>

          <div className="mt-4 rounded-xl border border-border-hairline bg-bg-page/60 p-3">
            <div className="flex flex-col gap-2 sm:flex-row sm:items-center sm:justify-between">
              <Link
                to="/thread/$threadId"
                params={{ threadId: item.context.thread_id }}
                search={{ from: undefined }}
                className="min-w-0 truncate font-semibold text-ink-primary focus-ring outline-none hover:text-accent-blue"
              >
                {item.context.subject || '(no subject)'}
              </Link>
              <time className="shrink-0 text-xs text-ink-tertiary">
                {formatDateTime(item.context.received_at)}
              </time>
            </div>
            <p className="mt-1 truncate text-sm text-ink-secondary">
              {item.context.from || 'Unknown sender'}
            </p>
            <p className="mt-2 line-clamp-2 text-sm leading-6 text-ink-tertiary">
              {item.context.preview || 'No message preview available.'}
            </p>
          </div>
        </div>
      </div>
    </article>
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
    <div className="space-y-3">
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
      list={<div className="space-y-5">{list}</div>}
      wide
    />
  );
}
