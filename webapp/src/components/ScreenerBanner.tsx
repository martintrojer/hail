import { Link } from '@tanstack/react-router';
import { Alert, AlertDescription } from './ui/alert';
import { Button } from './ui/button';
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
    <Alert className="mb-4 flex w-full items-center justify-between gap-4">
      <div className="flex min-w-0 items-center gap-3">
        <ScreenerIcon
          {...iconSizeProps.sm}
          className="shrink-0 text-muted-foreground"
          aria-hidden="true"
        />
        <AlertDescription className="text-sm text-foreground sm:text-base">
          {pendingCount} new {senderLabel} waiting
        </AlertDescription>
      </div>
      <Button asChild variant="outline" size="sm" className="shrink-0">
        <Link to="/screener">
          Screen them
        </Link>
      </Button>
    </Alert>
  );
}
