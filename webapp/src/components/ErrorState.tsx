import { AlertTriangle } from 'lucide-react';
import { Alert, AlertAction, AlertDescription, AlertTitle } from './ui/alert';
import { Button } from './ui/button';

interface ErrorStateProps {
  onRetry?: () => void;
  message?: string;
}

export function ErrorState({ onRetry, message }: ErrorStateProps) {
  return (
    <div className="flex min-h-[300px] items-center justify-center p-4">
      <Alert variant="destructive" className="max-w-md">
        <AlertTriangle aria-hidden="true" />
        <AlertTitle>Something went wrong.</AlertTitle>
        {message ? <AlertDescription>{message}</AlertDescription> : null}
        {onRetry ? (
          <AlertAction>
            <Button type="button" size="sm" variant="outline" onClick={onRetry}>
              Retry
            </Button>
          </AlertAction>
        ) : null}
      </Alert>
    </div>
  );
}
