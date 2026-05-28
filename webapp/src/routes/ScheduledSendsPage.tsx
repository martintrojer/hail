import { useApiClient } from '../api/ApiClientProvider';
import {
  type HailApiClient,
  type ScheduledSend,
} from '../api/client';
import {
  useCancelScheduledSendMutation,
  useScheduledSends,
} from '../api/query';
import { ActionableList } from '../components/ActionableList';
import { ErrorState } from '../components/ErrorState';
import { LoadingState } from '../components/LoadingState';
import { StateCard } from '../components/StateCard';
import { useUndoToast } from '../components/UndoToastProvider';
import { Badge } from '../components/ui/badge';
import { Button } from '../components/ui/button';
import { AppShell } from '../layout/AppShell';
import { formatDateTime } from '../lib/dates';
import { actionErrorMessage, viewErrorMessage } from '../lib/errorMessages';

interface ScheduledSendsPageProps {
  client?: HailApiClient;
}

const statusLabels: Record<string, string> = {
  pending: 'Pending',
  claimed: 'Sending',
  sent: 'Sent',
  failed: 'Failed',
  cancelled: 'Cancelled',
};

function statusLabel(status: string) {
  return statusLabels[status] ?? status;
}

function isCancellable(item: ScheduledSend) {
  return item.status === 'pending' || item.status === 'cancelled';
}

function ScheduledSendRow({ item, client }: { item: ScheduledSend; client: HailApiClient }) {
  const { showToast } = useUndoToast();
  const cancel = useCancelScheduledSendMutation(client, {
    onSuccess: (cancelled) => {
      showToast({
        message: cancelled.status === 'cancelled'
          ? 'Scheduled send cancelled.'
          : 'Scheduled send updated.',
      });
    },
    onError: (error) => {
      showToast({
        message: actionErrorMessage(error, 'Cancel scheduled send'),
      });
    },
  });
  const cancelDisabled = cancel.isPending || !isCancellable(item) || item.status === 'cancelled';

  return (
    <article className="group flex items-center gap-3 border-b border-border py-4 pl-3 pr-1 hover:bg-muted/50 focus-within:bg-muted/50 sm:py-5 sm:pr-0">
      <div
        className="min-w-0 flex-1 border-l-[3px] border-l-transparent pl-3 outline-none focus-visible:border-l-primary focus-visible:outline-none"
        tabIndex={0}
        data-hail-mail-list-item="true"
        data-hail-thread-id={String(item.id)}
        aria-label={`Scheduled send ${item.draft_email_id} for ${formatDateTime(item.send_at)}`}
      >
        <div className="flex flex-wrap items-center gap-x-3 gap-y-1">
          <p className="min-w-0 truncate text-base font-semibold text-foreground">
            Draft {item.draft_email_id}
          </p>
          <Badge variant="outline">
            {statusLabel(item.status)}
          </Badge>
        </div>
        <p className="mt-1 text-sm leading-snug text-muted-foreground">
          <time dateTime={item.send_at}>
            Sends at {formatDateTime(item.send_at)}
          </time>
        </p>
        <p className="mt-1 text-xs leading-snug text-muted-foreground">
          Created {formatDateTime(item.created_at)}
        </p>
        {item.error ? (
          <p role="alert" className="mt-2 text-sm text-destructive">
            {item.error}
          </p>
        ) : null}
      </div>
      <Button
        type="button"
        variant="outline"
        size="xs"
        className="shrink-0 opacity-90 sm:opacity-0 sm:group-hover:opacity-100 sm:group-focus-within:opacity-100"
        onClick={() => cancel.mutate(item.id)}
        disabled={cancelDisabled}
        title={cancel.error ? actionErrorMessage(cancel.error, 'Cancel scheduled send') : undefined}
      >
        {cancel.isPending ? 'Cancelling…' : item.status === 'cancelled' ? 'Cancelled' : 'Cancel'}
      </Button>
    </article>
  );
}

function EmptyScheduledSendsState() {
  return (
    <StateCard
      title="No scheduled sends."
      body="Messages scheduled from the composer will appear here until they are sent or cancelled."
    />
  );
}

export function ScheduledSendsPage({ client }: ScheduledSendsPageProps) {
  const contextClient = useApiClient();
  const apiClient = client ?? contextClient;
  const query = useScheduledSends(apiClient);

  let list;
  if (query.isPending) {
    list = <LoadingState label="Loading scheduled sends" />;
  } else if (query.isError) {
    list = (
      <ErrorState
        message={viewErrorMessage(query.error, 'Scheduled sends')}
        onRetry={() => void query.refetch()}
      />
    );
  } else {
    const pendingItems = query.data.filter((item) => item.status === 'pending');
    list = (
      <ActionableList
        items={pendingItems}
        actions={{ availableActions: [] }}
        renderItem={(item) => <ScheduledSendRow item={item} client={apiClient} />}
        keyExtractor={(item) => String(item.id)}
        emptyState={<EmptyScheduledSendsState />}
      />
    );
  }

  return (
    <AppShell
      title="Scheduled"
      description="Messages waiting for scheduled delivery. Cancel anything you no longer want sent."
      list={list}
    />
  );
}
