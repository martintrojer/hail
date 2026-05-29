import { HailApiError, type HailApiClient, type LabelThreadItem } from '../api/client';
import { useLabelThreads } from '../api/query';
import { useApiClient } from '../api/ApiClientProvider';
import { ActionableList } from '../components/ActionableList';
import { ErrorState } from '../components/ErrorState';
import { LoadingState } from '../components/LoadingState';
import { MailRow } from '../components/MailRow';
import { StateCard } from '../components/StateCard';
import { ThreadLink } from '../components/ThreadLink';
import { Badge } from '../components/ui/badge';
import { AppShell } from '../layout/AppShell';
import { cn } from '../lib/utils';
import { viewErrorMessage } from '../lib/errorMessages';

interface LabelViewPageProps {
  labelId: number;
  client?: HailApiClient;
}

function labelPath(name: string, pathSegments: string[] = []) {
  const segments = pathSegments.length > 0 ? pathSegments : name.split('/');
  return segments.map((segment) => segment.trim()).filter(Boolean).join(' / ') || name;
}

function LabelThreadRow({
  item,
  selected,
  onToggleSelect,
}: {
  item: LabelThreadItem;
  selected?: boolean;
  onToggleSelect?: () => void;
}) {
  return (
    <ThreadLink
      threadId={item.thread_id}
      mailListItem
      className={cn(
        'block border-b border-l-2 border-b-border border-l-transparent py-1 focus-visible:border-l-primary focus-visible:bg-accent focus-visible:outline-none hover:bg-muted/60',
        selected && 'bg-accent',
      )}
      ariaLabel={`Open ${item.subject || 'thread'} from ${item.from || 'unknown sender'}`}
    >
      <MailRow
        from={item.from || 'Unknown sender'}
        subject={item.subject || '(no subject)'}
        preview={item.preview || 'No preview available.'}
        receivedAt={undefined}
        receivedAtFallback=""
        selected={selected}
        onToggleSelect={onToggleSelect}
        labels={item.labels}
      />
    </ThreadLink>
  );
}

function labelViewErrorMessage(error: Error) {
  if (error instanceof HailApiError && error.status === 404) {
    return 'This label was not found. It may have been renamed or deleted.';
  }

  return viewErrorMessage(error, 'Mail view');
}

export function LabelViewPage({ labelId, client }: LabelViewPageProps) {
  const contextClient = useApiClient();
  const apiClient = client ?? contextClient;
  const validLabelId = Number.isSafeInteger(labelId) && labelId > 0;
  const query = useLabelThreads(labelId, apiClient, { enabled: validLabelId });

  if (!validLabelId) {
    return (
      <AppShell
        title="Label not found"
        list={
          <StateCard
            title="Label not found"
            body="This label link is invalid. Choose a label from the sidebar or search."
          />
        }
      />
    );
  }

  const title = query.data ? labelPath(query.data.label.name, query.data.label.path_segments) : 'Label';
  const actions = query.data ? (
    <Badge variant="secondary">
      {query.data.label.thread_count} {query.data.label.thread_count === 1 ? 'thread' : 'threads'}
    </Badge>
  ) : null;

  let list;
  if (query.isPending) {
    list = <LoadingState label="Loading label mail" />;
  } else if (query.isError) {
    list = (
      <ErrorState
        message={labelViewErrorMessage(query.error)}
        onRetry={() => void query.refetch()}
      />
    );
  } else {
    list = (
      <ActionableList
        items={query.data.items}
        actions={{
          client: apiClient,
          availableActions: ['archive', 'trash', 'set-aside', 'reply-later', 'classify'],
        }}
        renderItem={(item, { selected, onToggleSelect }) => (
          <LabelThreadRow
            item={item}
            selected={selected}
            onToggleSelect={onToggleSelect}
          />
        )}
        keyExtractor={(item) => item.thread_id}
        emptyState={
          <StateCard
            title="No mail with this label yet."
            body="Threads assigned to this label will appear here."
          />
        }
      />
    );
  }

  return (
    <AppShell
      title={title}
      actions={actions}
      list={list}
    />
  );
}
