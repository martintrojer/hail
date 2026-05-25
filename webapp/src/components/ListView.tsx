import { Fragment, useEffect, useRef, type ReactNode } from 'react';

interface ListViewProps<T> {
  items: T[];
  renderItem: (item: T, index: number) => ReactNode;
  keyExtractor: (item: T) => string;
  hasMore: boolean;
  isLoadingMore: boolean;
  onLoadMore: () => void;
  emptyState: ReactNode;
}

export function ListView<T>({
  items,
  renderItem,
  keyExtractor,
  hasMore,
  isLoadingMore,
  onLoadMore,
  emptyState,
}: ListViewProps<T>) {
  const sentinelRef = useRef<HTMLDivElement | null>(null);
  const onLoadMoreRef = useRef(onLoadMore);

  useEffect(() => {
    onLoadMoreRef.current = onLoadMore;
  }, [onLoadMore]);

  useEffect(() => {
    const sentinel = sentinelRef.current;
    if (!sentinel) {
      return;
    }

    if (typeof IntersectionObserver === 'undefined') {
      return;
    }

    const observer = new IntersectionObserver((entries) => {
      const entry = entries[0];
      if (entry?.isIntersecting && hasMore && !isLoadingMore) {
        onLoadMoreRef.current();
      }
    });

    observer.observe(sentinel);

    return () => observer.disconnect();
  }, [hasMore, isLoadingMore]);

  if (items.length === 0 && !isLoadingMore) {
    return <>{emptyState}</>;
  }

  return (
    <>
      {items.map((item, index) => (
        <Fragment key={keyExtractor(item)}>{renderItem(item, index)}</Fragment>
      ))}

      {isLoadingMore ? (
        <div
          role="status"
          aria-label="Loading more"
          className="flex items-center justify-center py-5 text-sm text-ink-tertiary"
        >
          <span className="h-4 w-4 animate-spin rounded-full border-2 border-border-hairline border-t-ink-tertiary" />
        </div>
      ) : null}

      {!hasMore && items.length > 0 ? (
        <div className="py-6 text-center">
          <div className="border-t border-border-hairline" />
          <p className="mt-4 text-xs text-ink-tertiary">You&apos;re all caught up</p>
        </div>
      ) : null}

      <div ref={sentinelRef} aria-hidden="true" className="h-px" />
    </>
  );
}
