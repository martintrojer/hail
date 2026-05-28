import { useEffect, useState } from 'react';
import { cn } from '../lib/utils';
import { Skeleton } from './ui/skeleton';

interface LoadingStateProps {
  className?: string;
  label?: string;
}

export function LoadingState({
  className = '',
  label = 'Loading',
}: LoadingStateProps) {
  const [showLoading, setShowLoading] = useState(false);

  useEffect(() => {
    const timeout = window.setTimeout(() => setShowLoading(true), 1000);

    return () => window.clearTimeout(timeout);
  }, []);

  return (
    <div
      role="status"
      aria-label={label}
      className={cn(
        'flex min-h-[300px] flex-col items-center justify-center gap-3 p-4 text-center text-sm text-muted-foreground',
        className,
      )}
    >
      {showLoading ? (
        <>
          <Skeleton className="h-4 w-32" />
          <Skeleton className="h-3 w-48" />
          <span className="sr-only">Loading…</span>
        </>
      ) : null}
    </div>
  );
}
