import { useEffect, useState } from 'react';

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
      className={[
        'flex min-h-[300px] flex-col items-center justify-center text-center text-sm text-ink-tertiary',
        className,
      ].filter(Boolean).join(' ')}
    >
      {showLoading ? 'Loading…' : null}
    </div>
  );
}
