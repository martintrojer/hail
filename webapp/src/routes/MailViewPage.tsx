import { Link } from '@tanstack/react-router';
import type { ReactNode } from 'react';
import { HailApiError, type MailViewItem, type MailViewKind } from '../api/client';
import { useFeedView, useImboxView, usePapertrailView } from '../api/query';
import { AppShell } from '../layout/AppShell';

interface MailViewPageProps {
  view: MailViewKind;
  title: string;
  description: string;
}

const viewLabels: Record<MailViewKind, string> = {
  imbox: 'Imbox',
  feed: 'Feed',
  papertrail: 'Paper Trail',
};

function useMailView(view: MailViewKind) {
  switch (view) {
    case 'imbox':
      return useImboxView();
    case 'feed':
      return useFeedView();
    case 'papertrail':
      return usePapertrailView();
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

  return (
    <div className={view === 'feed' ? 'space-y-4' : 'space-y-3'}>
      {Array.from({ length: rows }, (_, index) => (
        <div
          key={index}
          className="animate-pulse rounded-2xl border border-slate-800 bg-slate-900/60 p-4"
        >
          <div className="h-4 w-2/3 rounded bg-slate-800" />
          <div className="mt-3 h-3 w-full rounded bg-slate-800" />
          <div className="mt-2 h-3 w-1/2 rounded bg-slate-800" />
        </div>
      ))}
    </div>
  );
}

function StateCard({ title, body }: { title: string; body: string }) {
  return (
    <div className="flex min-h-64 flex-col items-center justify-center rounded-2xl border border-dashed border-slate-800 bg-slate-900/40 p-8 text-center">
      <p className="text-base font-semibold text-slate-200">{title}</p>
      <p className="mt-2 max-w-sm text-sm text-slate-400">{body}</p>
    </div>
  );
}

function ThreadCard({ item, view }: { item: MailViewItem; view: MailViewKind }) {
  if (view === 'feed') {
    return <FeedThreadCard item={item} />;
  }

  if (view === 'papertrail') {
    return <PaperTrailThreadCard item={item} />;
  }

  return <ImboxThreadCard item={item} />;
}

function UnreadDot({ unread }: { unread: boolean }) {
  return unread ? (
    <span className="mt-1.5 h-2.5 w-2.5 shrink-0 rounded-full bg-sky-300 shadow shadow-sky-400/50" />
  ) : (
    <span className="mt-1.5 h-2.5 w-2.5 shrink-0 rounded-full border border-slate-700" />
  );
}

function ClassificationPill({ item }: { item: MailViewItem }) {
  return (
    <span className="rounded-full border border-slate-700 bg-slate-950/80 px-2 py-0.5 text-[0.7rem] font-semibold uppercase tracking-wide text-slate-300">
      {classificationLabel(item.classification)}
    </span>
  );
}

function ImboxThreadCard({ item }: { item: MailViewItem }) {
  return (
    <ThreadLink
      item={item}
      className="group flex gap-3 rounded-2xl border border-slate-800 bg-slate-900/70 p-4 transition hover:border-sky-500/60 hover:bg-slate-900 focus:outline-none focus:ring-2 focus:ring-sky-400"
    >
      <UnreadDot unread={item.unread} />
      <div className="min-w-0 flex-1">
        <div className="flex items-start justify-between gap-3">
          <p className="truncate text-sm font-semibold text-slate-100">
            {item.from || 'Unknown sender'}
          </p>
          <time className="shrink-0 text-xs text-slate-500">
            {formatDate(item.received_at)}
          </time>
        </div>
        <p className="mt-1 truncate text-base font-semibold text-slate-200">
          {item.subject || '(no subject)'}
        </p>
        <p className="mt-1 line-clamp-2 text-sm text-slate-400">
          {item.preview || 'No preview available.'}
        </p>
        <div className="mt-3 flex items-center justify-between gap-2">
          <ClassificationPill item={item} />
          {item.unread ? (
            <span className="text-xs font-semibold text-sky-200">Unread</span>
          ) : null}
        </div>
      </div>
    </ThreadLink>
  );
}

function FeedThreadCard({ item }: { item: MailViewItem }) {
  return (
    <div className="relative pl-6 before:absolute before:left-2 before:top-0 before:h-full before:w-px before:bg-slate-800">
      <UnreadDot unread={item.unread} />
      <ThreadLink
        item={item}
        className="group -mt-4 block rounded-3xl border border-slate-800 bg-slate-900/60 p-5 transition hover:border-emerald-400/60 hover:bg-slate-900 focus:outline-none focus:ring-2 focus:ring-emerald-400"
      >
        <div className="flex items-start justify-between gap-3">
          <div className="min-w-0">
            <p className="truncate text-sm font-semibold text-emerald-100">
              {item.from || 'Unknown sender'}
            </p>
            <p className="mt-2 text-lg font-semibold leading-snug text-slate-100">
              {item.subject || '(no subject)'}
            </p>
          </div>
          <time className="shrink-0 rounded-full bg-slate-950 px-2 py-1 text-xs text-slate-400">
            {formatDate(item.received_at)}
          </time>
        </div>
        <p className="mt-3 line-clamp-3 text-sm leading-6 text-slate-300">
          {item.preview || 'No preview available.'}
        </p>
        <div className="mt-4 flex items-center gap-2">
          <ClassificationPill item={item} />
          {item.unread ? (
            <span className="rounded-full bg-emerald-400/10 px-2 py-0.5 text-xs font-semibold text-emerald-200">
              New
            </span>
          ) : null}
        </div>
      </ThreadLink>
    </div>
  );
}

function PaperTrailThreadCard({ item }: { item: MailViewItem }) {
  return (
    <ThreadLink
      item={item}
      className="group grid grid-cols-[auto_minmax(0,1fr)_auto] items-start gap-3 rounded-xl border border-slate-800 bg-slate-900/50 px-3 py-2.5 transition hover:border-amber-400/60 hover:bg-slate-900 focus:outline-none focus:ring-2 focus:ring-amber-400"
    >
      <UnreadDot unread={item.unread} />
      <div className="min-w-0">
        <div className="flex items-center gap-2">
          <p className="truncate text-sm font-semibold text-slate-100">
            {item.from || 'Unknown sender'}
          </p>
          <ClassificationPill item={item} />
        </div>
        <p className="mt-0.5 truncate text-sm text-slate-300">
          {item.subject || '(no subject)'}
        </p>
        <p className="mt-0.5 truncate text-xs text-slate-500">
          {item.preview || 'No preview available.'}
        </p>
      </div>
      <time className="text-right text-xs text-slate-500">
        {formatDate(item.received_at)}
      </time>
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
      aria-label={`Open ${item.subject || 'thread'} from ${item.from || 'unknown sender'}`}
    >
      {children}
    </Link>
  );
}

export function MailViewPage({ view, title, description }: MailViewPageProps) {
  const query = useMailView(view);

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
      <div className={view === 'feed' ? 'space-y-5' : view === 'papertrail' ? 'space-y-2' : 'space-y-3'}>
        {query.data.items.map((item) => (
          <ThreadCard key={`${item.thread_id}:${item.email_id}`} item={item} view={view} />
        ))}
      </div>
    );
  }

  return <AppShell title={title} description={description} list={list} />;
}
