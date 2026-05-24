import { Link } from '@tanstack/react-router';
import type { ReactNode } from 'react';
import { HailApiError, type HailApiClient, type MailViewItem, type MailViewKind } from '../api/client';
import { useFeedView, useImboxView, usePapertrailView } from '../api/query';
import { ScreenerBanner } from '../components/ScreenerBanner';
import { AppShell } from '../layout/AppShell';

interface MailViewPageProps {
  view: MailViewKind;
  title: string;
  description: string;
  client?: HailApiClient;
}

const viewLabels: Record<MailViewKind, string> = {
  imbox: 'Imbox',
  feed: 'Feed',
  papertrail: 'Paper Trail',
};

function useMailView(view: MailViewKind, client?: HailApiClient) {
  switch (view) {
    case 'imbox':
      return useImboxView(client);
    case 'feed':
      return useFeedView(client);
    case 'papertrail':
      return usePapertrailView(client);
  }
}

function errorMessage(error: Error) {
  if (error instanceof HailApiError) {
    if (error.status === 401) {
      return 'Your session expired. Sign in again to refresh this view.';
    }
    return `Mail view failed with HTTP ${error.status}.`;
  }

  return 'Mail view failed to load. Refresh and try again.';
}

function formatDate(value: string | null | undefined) {
  if (!value) {
    return 'No date';
  }

  const date = new Date(value);
  if (Number.isNaN(date.valueOf())) {
    return value;
  }

  const now = new Date();
  const sameYear = date.getFullYear() === now.getFullYear();
  return new Intl.DateTimeFormat(undefined, {
    month: 'short',
    day: 'numeric',
    ...(sameYear ? {} : { year: 'numeric' }),
    hour: 'numeric',
    minute: '2-digit',
  }).format(date);
}

function classificationLabel(classification: MailViewItem['classification']) {
  return viewLabels[classification];
}

function SkeletonList({ view }: { view: MailViewKind }) {
  const rows = view === 'papertrail' ? 8 : 5;
  const rowClassName =
    view === 'feed'
      ? 'animate-pulse border-b border-border-hairline py-6 sm:py-7'
      : view === 'papertrail'
        ? 'animate-pulse border-b border-border-hairline py-2.5 sm:py-3'
        : 'animate-pulse border-b border-border-hairline py-4 sm:py-5';

  return (
    <div aria-label={`Loading ${viewLabels[view]} mail`}>
      {Array.from({ length: rows }, (_, index) => (
        <div
          key={index}
          className={rowClassName}
        >
          <div className="flex items-center justify-between gap-4">
            <div className="h-4 w-40 rounded bg-border-hairline" />
            <div className="h-3 w-16 rounded bg-border-hairline" />
          </div>
          <div className="mt-2 h-4 w-2/3 rounded bg-border-hairline" />
          {view === 'papertrail' ? null : (
            <div className="mt-2 h-3 w-full rounded bg-border-hairline" />
          )}
        </div>
      ))}
    </div>
  );
}

function StateCard({ title, body }: { title: string; body: string }) {
  return (
    <div className="flex min-h-64 flex-col items-center justify-center border-y border-border-hairline p-8 text-center">
      <p className="text-base font-semibold text-ink-primary">{title}</p>
      <p className="mt-2 max-w-sm text-sm leading-6 text-ink-secondary">{body}</p>
    </div>
  );
}

function ThreadCard({ item, view }: { item: MailViewItem; view: MailViewKind }) {
  return <MailThreadRow item={item} view={view} />;
}

function ScreenReaderThreadMetadata({ item }: { item: MailViewItem }) {
  return (
    <span className="sr-only">
      <span>{classificationLabel(item.classification)}</span>
      <span role="img" aria-label={item.unread ? 'Unread thread' : 'Read thread'} />
      {item.unread ? <span>Unread</span> : null}
    </span>
  );
}

function NewPill() {
  return (
    <span className="shrink-0 rounded-full bg-accent-yellow px-2 py-0.5 text-[0.7rem] font-semibold uppercase leading-tight tracking-wider text-ink-primary">
      New
    </span>
  );
}

function MailThreadRow({ item, view }: { item: MailViewItem; view: MailViewKind }) {
  if (view === 'feed') {
    return <FeedThreadRow item={item} />;
  }

  if (view === 'papertrail') {
    return <PaperTrailThreadRow item={item} />;
  }

  return <ImboxThreadRow item={item} />;
}

function ImboxThreadRow({ item }: { item: MailViewItem }) {
  return (
    <ThreadLink
      item={item}
      className="block border-b border-l-[3px] border-b-border-hairline border-l-transparent py-4 pl-3 pr-0 hover:bg-bg-hover focus-visible:border-l-accent-blue focus-visible:bg-bg-selected focus-visible:outline-none sm:py-5"
    >
      <ScreenReaderThreadMetadata item={item} />
      <div className="flex items-baseline justify-between gap-4">
        <div className="flex min-w-0 items-center gap-2">
          <p
            className={`truncate text-base leading-snug text-ink-primary ${
              item.unread ? 'font-bold' : 'font-semibold'
            }`}
          >
            {item.from || 'Unknown sender'}
          </p>
          {item.unread ? <NewPill /> : null}
        </div>
        <time className="shrink-0 text-sm leading-snug text-ink-tertiary">
          {formatDate(item.received_at)}
        </time>
      </div>
      <p className="mt-1 truncate text-[0.95rem] font-normal leading-snug text-ink-secondary">
        {item.subject || '(no subject)'}
      </p>
      <p className="mt-1 truncate text-sm font-normal leading-snug text-ink-tertiary">
        {item.preview || 'No preview available.'}
      </p>
    </ThreadLink>
  );
}

function FeedThreadRow({ item }: { item: MailViewItem }) {
  return (
    <ThreadLink
      item={item}
      className="block border-b border-l-[3px] border-b-border-hairline border-l-transparent py-6 pl-3 pr-0 hover:bg-bg-hover focus-visible:border-l-accent-blue focus-visible:bg-bg-selected focus-visible:outline-none sm:py-7"
    >
      <ScreenReaderThreadMetadata item={item} />
      <div className="flex items-baseline justify-between gap-4">
        <div className="flex min-w-0 items-center gap-2">
          <p
            className={`truncate text-[1.05rem] leading-snug text-ink-primary ${
              item.unread ? 'font-bold' : 'font-semibold'
            }`}
          >
            {item.from || 'Unknown sender'}
          </p>
          {item.unread ? <NewPill /> : null}
        </div>
        <time className="shrink-0 text-sm leading-snug text-ink-tertiary">
          {formatDate(item.received_at)}
        </time>
      </div>
      <p className="mt-1 text-base font-normal leading-snug text-ink-primary">
        {item.subject || '(no subject)'}
      </p>
      <p className="mt-2 line-clamp-3 text-sm font-normal leading-6 text-ink-secondary">
        {item.preview || 'No preview available.'}
      </p>
    </ThreadLink>
  );
}

function PaperTrailThreadRow({ item }: { item: MailViewItem }) {
  return (
    <ThreadLink
      item={item}
      className="block border-b border-l-[3px] border-b-border-hairline border-l-transparent py-2.5 pl-3 pr-0 hover:bg-bg-hover focus-visible:border-l-accent-blue focus-visible:bg-bg-selected focus-visible:outline-none sm:py-3"
    >
      <ScreenReaderThreadMetadata item={item} />
      <div className="flex items-baseline justify-between gap-4">
        <div className="min-w-0 sm:flex sm:items-baseline sm:gap-2">
          <p className="truncate text-[0.95rem] font-semibold leading-snug text-ink-primary">
            {item.from || 'Unknown sender'}
          </p>
          <p className="mt-0.5 truncate text-[0.95rem] font-normal leading-snug text-ink-secondary sm:mt-0">
            {item.subject || '(no subject)'}
          </p>
        </div>
        <time className="shrink-0 text-[0.8rem] leading-snug text-ink-tertiary">
          {formatDate(item.received_at)}
        </time>
      </div>
    </ThreadLink>
  );
}

function ThreadLink({
  item,
  className,
  children,
}: {
  item: MailViewItem;
  className: string;
  children: ReactNode;
}) {
  return (
    <Link
      to="/thread/$threadId"
      params={{ threadId: item.thread_id }}
      className={className}
      data-hail-mail-list-item="true"
      aria-label={`Open ${item.subject || 'thread'} from ${item.from || 'unknown sender'}`}
    >
      {children}
    </Link>
  );
}

export function MailViewPage({
  view,
  title,
  description,
  client,
}: MailViewPageProps) {
  const query = useMailView(view, client);
  const pendingCount = 0;

  let list;
  if (query.isPending) {
    list = <SkeletonList view={view} />;
  } else if (query.isError) {
    list = (
      <StateCard
        title="Could not load mail"
        body={errorMessage(query.error)}
      />
    );
  } else if (query.data.items.length === 0) {
    list = (
      <StateCard
        title={`No ${title} mail yet`}
        body={`When the server classifies threads as ${title}, they will show up here.`}
      />
    );
  } else {
    list = (
      <div>
        {view === 'imbox' ? <ScreenerBanner pendingCount={pendingCount} /> : null}
        <div>
          {query.data.items.map((item) => (
            <ThreadCard key={`${item.thread_id}:${item.email_id}`} item={item} view={view} />
          ))}
        </div>
      </div>
    );
  }

  return <AppShell title={title} description={description} list={list} />;
}
