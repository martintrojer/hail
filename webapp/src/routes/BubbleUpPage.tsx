import { Link } from '@tanstack/react-router';
import { type BubbleUpViewItem } from '../api/client';
import { useBubbleUpView, useCancelBubbleUpMutation } from '../api/query';
import { ErrorState } from '../components/ErrorState';
import { ListView } from '../components/ListView';
import { LoadingState } from '../components/LoadingState';
import { useUndoToast } from '../components/UndoToastProvider';
import { AppShell } from '../layout/AppShell';
import { senderNameClass } from '../lib/mailRowStyles';

function formatBubbleTime(value: string) {
  const date = new Date(value);
  if (Number.isNaN(date.valueOf())) {
    return value;
  }

  return new Intl.DateTimeFormat(undefined, {
    month: 'short',
    day: 'numeric',
    year: date.getFullYear() === new Date().getFullYear() ? undefined : 'numeric',
    hour: 'numeric',
    minute: '2-digit',
  }).format(date);
}

function EmptyBubbleUpState() {
  return (
    <div className="flex min-h-[300px] flex-col items-center justify-center p-8 text-center">
      <p className="text-lg font-semibold text-ink-primary">No bubble-ups scheduled.</p>
      <p className="mt-2 max-w-sm text-sm leading-6 text-ink-secondary">
        Bubble up a thread and its return time will show here.
      </p>
    </div>
  );
}

function BubbleUpRow({ item }: { item: BubbleUpViewItem }) {
  const { showToast } = useUndoToast();
  const cancel = useCancelBubbleUpMutation(undefined, {
    onSuccess: () => {
      showToast({ message: 'Bubble-up cancelled.' });
    },
    onError: () => {
      showToast({ message: 'Cancel failed. Refresh and try again.' });
    },
  });

  return (
    <div className="group flex items-center gap-3 border-b border-border-hairline py-4 pl-3 pr-1 hover:bg-bg-hover focus-within:bg-bg-selected sm:py-5 sm:pr-0">
      <Link
        to="/thread/$threadId"
        search={{ from: undefined }} params={{ threadId: item.thread_id }}
        className="min-w-0 flex-1 border-l-[3px] border-l-transparent pl-3 outline-none focus-visible:border-l-accent-blue focus-visible:outline-none"
        aria-label={`Open ${item.subject || 'thread'} from ${item.from || 'unknown sender'}`}
      >
        <p className={senderNameClass}>
          {item.from || 'Unknown sender'}
        </p>
        <p className="mt-1 truncate text-[0.95rem] font-normal leading-snug text-ink-secondary">
          {item.subject || '(no subject)'}
        </p>
        <p className="mt-1 text-sm leading-snug text-ink-tertiary">
          <time dateTime={item.surface_at}>
            Bubbles up at {formatBubbleTime(item.surface_at)}
          </time>
        </p>
      </Link>
      <button
        type="button"
        className="shrink-0 rounded-full border border-border-menu px-3 py-1 text-xs font-semibold text-ink-secondary opacity-90 focus-ring outline-none hover:bg-bg-selected hover:text-accent-blue disabled:cursor-not-allowed disabled:opacity-60 sm:opacity-0 sm:group-hover:opacity-100 sm:group-focus-within:opacity-100"
        onClick={() => cancel.mutate({ threadId: item.thread_id })}
        disabled={cancel.isPending}
      >
        {cancel.isPending ? 'Cancelling…' : 'Cancel'}
      </button>
    </div>
  );
}

function BubbleUpList() {
  const query = useBubbleUpView();

  if (query.isPending) {
    return <LoadingState />;
  }

  if (query.isError) {
    return (
      <ErrorState
        message="Bubble Up failed to load. Refresh and try again."
        onRetry={() => void query.refetch()}
      />
    );
  }

  return (
    <ListView
      items={query.data.items}
      renderItem={(item) => <BubbleUpRow item={item} />}
      keyExtractor={(item) => String(item.bubble_id)}
      hasMore={false}
      isLoadingMore={false}
      onLoadMore={() => {}}
      emptyState={<EmptyBubbleUpState />}
    />
  );
}

export function BubbleUpPage() {
  return (
    <AppShell
      title="Bubble Up"
      description="Threads scheduled to return to your attention."
      list={<BubbleUpList />}
    />
  );
}
