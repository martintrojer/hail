import { Link } from '@tanstack/react-router';
import { type PileItem } from '../api/client';
import { useReplyLaterView, useSetAsideView } from '../api/query';

type PileStackKind = 'set-aside' | 'reply-later';

interface StackConfig {
  kind: PileStackKind;
  title: string;
  accentClassName: string;
  hoverClassName: string;
  items: PileItem[];
}

function formatPileDate(value: string) {
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

function previewText(preview: PileItem['preview']): string | null {
  if (preview === null || preview === undefined) {
    return null;
  }

  if (typeof preview === 'string') {
    return preview.trim() || null;
  }

  if (typeof preview === 'number' || typeof preview === 'boolean') {
    return String(preview);
  }

  if (Array.isArray(preview)) {
    return preview.map(previewText).filter(Boolean).join(' · ') || null;
  }

  if (typeof preview === 'object') {
    const record = preview as Record<string, unknown>;
    const candidates = [
      record.preview,
      record.snippet,
      record.subject,
      record.title,
      record.from,
    ];

    for (const candidate of candidates) {
      const text = previewText(candidate);
      if (text) {
        return text;
      }
    }
  }

  return null;
}

function PileCard({ item, stack }: { item: PileItem; stack: StackConfig }) {
  const preview = previewText(item.preview);

  return (
    <Link
      to="/thread/$threadId"
      params={{ threadId: item.thread_id }}
      className={`group block rounded-2xl border border-border-menu bg-bg-surface p-3 shadow-lg shadow-ink-primary/10 focus:outline-none focus:ring-2 focus:ring-accent-blue ${stack.hoverClassName}`}
      aria-label={`Open thread ${item.thread_id} from ${stack.title}`}
    >
      <div className="flex items-start justify-between gap-3">
        <p className="min-w-0 truncate font-mono text-[0.72rem] font-semibold text-ink-primary">
          {item.thread_id}
        </p>
        <span className="shrink-0 rounded-full border border-border-hairline bg-bg-banner px-2 py-0.5 text-[0.65rem] font-semibold text-ink-secondary">
          #{item.position}
        </span>
      </div>

      {preview ? (
        <p className="mt-2 line-clamp-2 text-xs leading-5 text-ink-secondary">
          {preview}
        </p>
      ) : null}

      <time className="mt-2 block text-[0.68rem] text-ink-tertiary">
        Added {formatPileDate(item.added_at)}
      </time>
    </Link>
  );
}

function PileStack({ stack }: { stack: StackConfig }) {
  if (stack.items.length === 0) {
    return null;
  }

  const visibleItems = stack.items.slice(0, 4);
  const hiddenCount = stack.items.length - visibleItems.length;

  return (
    <section aria-label={stack.title}>
      <div className="mb-2 flex items-center justify-between gap-3 px-1">
        <h2 className="flex items-center gap-2 text-xs font-semibold uppercase tracking-[0.2em] text-ink-secondary">
          <span className={`h-2 w-2 rounded-full ${stack.accentClassName}`} />
          {stack.title}
        </h2>
        <span className="text-xs font-medium text-ink-tertiary">
          {stack.items.length}
        </span>
      </div>

      <div className="space-y-0">
        {visibleItems.map((item, index) => (
          <div
            key={`${stack.kind}:${item.thread_id}`}
            className={index === 0 ? undefined : '-mt-3'}
            style={{ marginLeft: `${index * 0.35}rem` }}
          >
            <PileCard item={item} stack={stack} />
          </div>
        ))}
      </div>

      {hiddenCount > 0 ? (
        <p className="mt-2 px-1 text-[0.68rem] text-ink-tertiary">
          +{hiddenCount} more in {stack.title}
        </p>
      ) : null}
    </section>
  );
}

export function Pile() {
  const setAside = useSetAsideView();
  const replyLater = useReplyLaterView();

  const stacks: StackConfig[] = [
    {
      kind: 'set-aside',
      title: 'Set Aside',
      accentClassName: 'bg-accent-blue shadow shadow-accent-blue/30',
      hoverClassName: 'hover:border-accent-blue hover:bg-bg-hover',
      items: setAside.data?.items ?? [],
    },
    {
      kind: 'reply-later',
      title: 'Reply Later',
      accentClassName: 'bg-accent-yellow shadow shadow-accent-yellow/30',
      hoverClassName: 'hover:border-accent-yellow hover:bg-bg-hover',
      items: replyLater.data?.items ?? [],
    },
  ];

  const totalItems = stacks.reduce((sum, stack) => sum + stack.items.length, 0);
  if (totalItems === 0) {
    return null;
  }

  return (
    <aside
      aria-label="Pile"
      className="fixed bottom-4 right-4 z-30 hidden w-[min(24rem,calc(100vw-2rem))] max-h-[calc(100vh-2rem)] overflow-y-auto rounded-3xl border border-border-menu bg-bg-surface p-3 text-ink-primary shadow-2xl shadow-ink-primary/15 md:block"
    >
      <div className="mb-3 flex items-center justify-between gap-3 px-1">
        <p className="text-sm font-semibold text-ink-primary">The Pile</p>
        <p className="text-xs text-ink-tertiary">{totalItems} stacked</p>
      </div>

      <div className="space-y-5">
        {stacks.map((stack) => (
          <PileStack key={stack.kind} stack={stack} />
        ))}
      </div>
    </aside>
  );
}
