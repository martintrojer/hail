import type { ReactNode } from 'react';

interface StateCardProps {
  title: string;
  body?: ReactNode;
  className?: string;
  bodyClassName?: string;
}

export function StateCard({
  title,
  body,
  className = 'flex min-h-[300px] flex-col items-center justify-center p-8 text-center',
  bodyClassName = 'mt-2 max-w-sm text-sm leading-6 text-ink-secondary',
}: StateCardProps) {
  return (
    <div className={className}>
      <p className="text-lg font-semibold text-ink-primary">{title}</p>
      {body ? <p className={bodyClassName}>{body}</p> : null}
    </div>
  );
}
