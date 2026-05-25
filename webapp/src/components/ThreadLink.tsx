import { Link } from '@tanstack/react-router';
import type { ReactNode } from 'react';

interface ThreadLinkProps {
  threadId: string;
  children: ReactNode;
  className?: string;
  ariaLabel?: string;
  mailListItem?: boolean;
}

export function ThreadLink({
  threadId,
  children,
  className,
  ariaLabel,
  mailListItem = false,
}: ThreadLinkProps) {
  return (
    <Link
      to="/thread/$threadId"
      search={{ from: undefined }}
      params={{ threadId }}
      className={className}
      data-hail-mail-list-item={mailListItem ? 'true' : undefined}
      data-hail-thread-id={mailListItem ? threadId : undefined}
      aria-label={ariaLabel}
    >
      {children}
    </Link>
  );
}
