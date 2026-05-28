import type { ReactNode } from 'react';
import { cn } from '../lib/utils';
import {
  Empty,
  EmptyDescription,
  EmptyHeader,
  EmptyTitle,
} from './ui/empty';

interface StateCardProps {
  title: string;
  body?: ReactNode;
  className?: string;
  bodyClassName?: string;
}

export function StateCard({
  title,
  body,
  className = '',
  bodyClassName,
}: StateCardProps) {
  return (
    <Empty className={cn('min-h-[300px]', className)}>
      <EmptyHeader>
        <EmptyTitle>{title}</EmptyTitle>
        {body ? (
          <EmptyDescription className={bodyClassName}>{body}</EmptyDescription>
        ) : null}
      </EmptyHeader>
    </Empty>
  );
}
