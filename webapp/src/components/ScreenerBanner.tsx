import { Link } from '@tanstack/react-router';
import { iconSizeProps, icons } from './icons';

interface ScreenerBannerProps {
  pendingCount: number;
}

export function ScreenerBanner({ pendingCount }: ScreenerBannerProps) {
  if (pendingCount <= 0) {
    return null;
  }

  const ScreenerIcon = icons.screenerShield;
  const senderLabel = pendingCount === 1 ? 'sender' : 'senders';

  return (
    <div className="mb-4 flex w-full items-center justify-between gap-4 rounded-lg bg-bg-banner px-4 py-3 text-ink-primary">
      <div className="flex min-w-0 items-center gap-3">
        <ScreenerIcon
          {...iconSizeProps.sm}
          className="shrink-0 text-ink-secondary"
          aria-hidden="true"
        />
        <p className="text-sm leading-5 sm:text-base">
          {pendingCount} new {senderLabel} waiting
        </p>
      </div>
      <Link
        to="/screener"
        className="shrink-0 text-sm font-semibold text-accent-blue transition hover:text-accent-blue-hover sm:text-base"
      >
        Screen them
      </Link>
    </div>
  );
}
