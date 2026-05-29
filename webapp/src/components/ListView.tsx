import { Fragment, useEffect, useRef, type ReactNode } from 'react';
import { Spinner } from './ui/spinner';

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
          className="flex items-center justify-center py-5 text-sm text-muted-foreground"
        >
          <Spinner aria-hidden="true" />
          <span className="sr-only">Loading more</span>
        </div>
      ) : null}

      <div ref={sentinelRef} aria-hidden="true" className="h-px" />
    </>
  );
}
