interface ErrorStateProps {
  onRetry?: () => void;
  message?: string;
}

export function ErrorState({ onRetry, message }: ErrorStateProps) {
  return (
    <div
      role="alert"
      className="flex min-h-[300px] flex-col items-center justify-center text-center"
    >
      <p className="text-base text-ink-primary">Something went wrong.</p>
      {message ? (
        <p className="mt-2 max-w-sm text-sm leading-6 text-ink-secondary">
          {message}
        </p>
      ) : null}
      {onRetry ? (
        <button
          type="button"
          onClick={onRetry}
          className="mt-2 text-sm font-semibold text-accent-blue outline-none hover:underline focus-visible:rounded-sm focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent-blue"
        >
          Retry
        </button>
      ) : null}
    </div>
  );
}
