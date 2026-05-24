import { useQueryClient } from '@tanstack/react-query';
import { Link } from '@tanstack/react-router';
import { useEffect, useRef, useState, type ComponentType, type MouseEvent } from 'react';
import type { PileItem } from '../api/client';
import { useClassifyThreadMutation, useReplyLaterView, useSetAsideView } from '../api/query';
import { queryKeys } from '../api/queryKeys';
import { Bookmark, Clock, X, iconSizeProps } from '../components/icons';
import { formatPileDate, pilePreview } from '../lib/pilePreview';

type PileStackKind = 'reply-later' | 'set-aside';

interface StackConfig {
  kind: PileStackKind;
  title: string;
  to: '/reply-later' | '/set-aside';
  Icon: ComponentType<{ className?: string; size?: number; strokeWidth?: number }>;
  items: PileItem[];
}

function CountBadge({ count }: { count: number }) {
  return (
    <span className="rounded-full bg-bg-banner px-2 py-0.5 text-xs font-semibold text-ink-primary ring-1 ring-border-hairline">
      {count}
    </span>
  );
}

function CollapsedRow({ stack }: { stack: StackConfig }) {
  return (
    <div className="flex items-center justify-between gap-3 rounded-md px-3 py-2 text-ink-primary">
      <span className="flex min-w-0 items-center gap-2 text-sm font-semibold">
        <stack.Icon className="shrink-0 text-ink-secondary" {...iconSizeProps.sm} />
        <span className="truncate">{stack.title}</span>
      </span>
      <CountBadge count={stack.items.length} />
    </div>
  );
}

function PileItemRow({ item, stack }: { item: PileItem; stack: StackConfig }) {
  const preview = pilePreview(item);
  const queryClient = useQueryClient();
  const release = useClassifyThreadMutation(undefined, {
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: queryKeys.views() });
    },
  });

  function removeFromPile(event: MouseEvent<HTMLButtonElement>) {
    event.preventDefault();
    event.stopPropagation();
    release.mutate({ threadId: item.thread_id, to: 'imbox' });
  }

  return (
    <Link
      to="/thread/$threadId"
      params={{ threadId: item.thread_id }}
      className="group flex items-start gap-2 rounded-md px-2 py-2 outline-none hover:bg-bg-hover focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent-blue"
      aria-label={`Open ${preview.subject} from ${preview.sender}`}
    >
      <span className="min-w-0 flex-1">
        <span className="block truncate text-sm font-semibold leading-snug text-ink-primary">
          {preview.sender}
        </span>
        <span className="block truncate text-[0.8rem] leading-snug text-ink-secondary">
          {preview.subject}
        </span>
        {stack.kind === 'reply-later' ? (
          <time className="mt-1 block truncate text-[0.72rem] leading-snug text-ink-tertiary">
            Deferred {formatPileDate(item.added_at)}
          </time>
        ) : null}
      </span>
      <button
        type="button"
        className="shrink-0 rounded-full p-1 text-ink-tertiary opacity-80 outline-none hover:bg-bg-selected hover:text-ink-primary focus-visible:opacity-100 focus-visible:outline-2 focus-visible:outline-accent-blue sm:opacity-0 sm:group-hover:opacity-100"
        onClick={removeFromPile}
        disabled={release.isPending}
        aria-label={`Move ${preview.subject} back to Imbox`}
        title="Move back to Imbox"
      >
        <X {...iconSizeProps.sm} aria-hidden="true" />
      </button>
    </Link>
  );
}

function ExpandedStack({ stack }: { stack: StackConfig }) {
  return (
    <section aria-label={stack.title} className="border-t border-border-hairline pt-3 first:border-t-0 first:pt-0">
      <div className="mb-1 flex items-center justify-between gap-3 px-2">
        <Link
          to={stack.to}
          className="flex items-center gap-2 text-sm font-semibold text-ink-primary outline-none hover:text-accent-blue focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent-blue"
        >
          <stack.Icon className="text-ink-secondary" {...iconSizeProps.sm} />
          {stack.title}
        </Link>
        <CountBadge count={stack.items.length} />
      </div>

      {stack.items.length === 0 ? (
        <p className="px-2 py-3 text-sm text-ink-tertiary">Nothing here.</p>
      ) : (
        <div className="space-y-0.5">
          {stack.items.map((item) => (
            <PileItemRow key={`${stack.kind}:${item.thread_id}`} item={item} stack={stack} />
          ))}
        </div>
      )}
    </section>
  );
}

export function Pile() {
  const [expanded, setExpanded] = useState(false);
  const pileRef = useRef<HTMLElement | null>(null);
  const setAside = useSetAsideView();
  const replyLater = useReplyLaterView();

  useEffect(() => {
    if (!expanded) {
      return undefined;
    }

    function onPointerDown(event: PointerEvent) {
      const target = event.target;
      if (target instanceof Node && !pileRef.current?.contains(target)) {
        setExpanded(false);
      }
    }

    function onKeyDown(event: KeyboardEvent) {
      if (event.key === 'Escape') {
        setExpanded(false);
      }
    }

    document.addEventListener('pointerdown', onPointerDown);
    document.addEventListener('keydown', onKeyDown);

    return () => {
      document.removeEventListener('pointerdown', onPointerDown);
      document.removeEventListener('keydown', onKeyDown);
    };
  }, [expanded]);

  const stacks: StackConfig[] = [
    {
      kind: 'reply-later',
      title: 'Reply Later',
      to: '/reply-later',
      Icon: Clock,
      items: replyLater.data?.items ?? [],
    },
    {
      kind: 'set-aside',
      title: 'Set Aside',
      to: '/set-aside',
      Icon: Bookmark,
      items: setAside.data?.items ?? [],
    },
  ];
  const totalItems = stacks.reduce((sum, stack) => sum + stack.items.length, 0);

  return (
    <aside
      ref={pileRef}
      aria-label="Pile"
      className="fixed bottom-4 right-4 z-50 w-[min(15rem,calc(100vw-2rem))] rounded-lg border border-border-menu bg-bg-surface text-ink-primary shadow-lg shadow-ink-primary/15"
    >
      {expanded ? (
        <div>
          <button
            type="button"
            className="flex w-full items-center justify-between gap-3 rounded-t-lg px-3 py-2.5 text-left outline-none hover:bg-bg-hover focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent-blue"
            onClick={() => setExpanded(false)}
            aria-expanded="true"
          >
            <span className="text-sm font-semibold">The Pile</span>
            <span className="text-xs font-medium text-ink-tertiary">{totalItems} stacked</span>
          </button>
          <div className="max-h-[min(28rem,calc(100vh-7rem))] space-y-3 overflow-y-auto p-2 pt-1">
            {stacks.map((stack) => (
              <ExpandedStack key={stack.kind} stack={stack} />
            ))}
          </div>
        </div>
      ) : (
        <button
          type="button"
          className="block w-full rounded-lg p-1 text-left outline-none hover:bg-bg-hover focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent-blue"
          onClick={() => setExpanded(true)}
          aria-expanded="false"
        >
          <CollapsedRow stack={stacks[0]} />
          <CollapsedRow stack={stacks[1]} />
        </button>
      )}
    </aside>
  );
}
