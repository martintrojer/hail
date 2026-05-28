import { useRef, useState } from 'react';
import { Link } from '@tanstack/react-router';
import { ShieldOff } from 'lucide-react';
import type { HailApiClient } from '../api/client';
import {
  type ScreenerClassification,
  type ScreenerPendingSender,
} from '../api/client';
import {
  useScreenerDecisionMutation,
  useScreenerView,
} from '../api/query';
import { ErrorState } from '../components/ErrorState';
import { LoadingState } from '../components/LoadingState';
import { ListView } from '../components/ListView';
import {
  ScreenerRoutingDropdown,
  type ScreenerRoutingDestination,
} from '../components/ScreenerRoutingDropdown';
import { useUndoToast } from '../components/UndoToastProvider';
import { Alert, AlertDescription } from '../components/ui/alert';
import { Badge } from '../components/ui/badge';
import { Button } from '../components/ui/button';
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from '../components/ui/collapsible';
import {
  Card,
  CardContent,
  CardFooter,
  CardHeader,
  CardTitle,
} from '../components/ui/card';
import { Empty, EmptyHeader, EmptyTitle } from '../components/ui/empty';
import { AppShell } from '../layout/AppShell';
import { formatDate } from '../lib/dates';
import { actionErrorMessage, viewErrorMessage } from '../lib/errorMessages';

function previewRecord(preview: unknown) {
  if (!preview || typeof preview !== 'object') {
    return null;
  }

  return preview as Record<string, unknown>;
}

function textFromKeys(record: Record<string, unknown> | null, keys: string[]) {
  if (!record) {
    return null;
  }

  for (const key of keys) {
    const value = record[key];
    if (typeof value === 'string' && value.trim().length > 0) {
      return value.trim();
    }
  }

  return null;
}

function subjectText(preview: unknown) {
  return textFromKeys(previewRecord(preview), ['subject', 'title']);
}

function previewText(preview: unknown) {
  if (typeof preview === 'string' && preview.trim().length > 0) {
    return preview.trim();
  }

  return textFromKeys(previewRecord(preview), [
    'text',
    'body',
    'preview',
    'snippet',
    'summary',
  ]);
}

function EmptyState() {
  return (
    <Empty className="min-h-[300px]">
      <EmptyHeader>
        <EmptyTitle>All clear. No one new is waiting.</EmptyTitle>
        <span className="sr-only">No unknown senders</span>
      </EmptyHeader>
    </Empty>
  );
}

function PendingSenderCard({
  sender,
  client,
}: {
  sender: ScreenerPendingSender;
  client?: HailApiClient;
}) {
  const [routingOpen, setRoutingOpen] = useState(false);
  const [routingAnchor, setRoutingAnchor] = useState<DOMRect | null>(null);
  const [expanded, setExpanded] = useState(false);
  const approveButtonRef = useRef<HTMLButtonElement | null>(null);
  const { showToast } = useUndoToast();
  const decision = useScreenerDecisionMutation(client, {
    onSuccess: (data, variables) => {
      if (variables.decision !== 'deny') {
        return;
      }

      showToast({
        message: `Denied ${variables.sender}.`,
        undo: data.undo ? { id: data.undo.id } : null,
        undoSuccessMessage: 'Sender decision undone.',
      });
    },
  });
  const isPending = decision.isPending;
  const senderIdentity = {
    name: sender.sender || 'Unknown sender',
    email: sender.sender || 'unknown address',
  };
  const subject = subjectText(sender.latest_preview) ?? 'First message from this sender';
  const preview =
    previewText(sender.latest_preview) ??
    'Preview unavailable until this message is indexed.';
  const emails = sender.emails ?? [];
  const expandedId = `screener-emails-${encodeURIComponent(sender.sender)}`;
  const pendingEmailCount = sender.message_count ?? emails.length;
  const emailCountLabel = `${pendingEmailCount} pending ${pendingEmailCount === 1 ? 'email' : 'emails'}`;

  function showRoutingDropdown() {
    if (approveButtonRef.current) {
      setRoutingAnchor(approveButtonRef.current.getBoundingClientRect());
    }
    setRoutingOpen(true);
  }

  function approve(destination: ScreenerRoutingDestination) {
    decision.mutate({
      sender: sender.sender,
      decision: 'approve',
      classify_as: destination as ScreenerClassification,
      apply_to_history: true,
    });
  }

  function deny() {
    decision.mutate({
      sender: sender.sender,
      decision: 'deny',
      apply_to_history: true,
    });
  }

  return (
    <Card size="sm">
      <Collapsible open={expanded} onOpenChange={setExpanded}>
        <CardHeader>
          <CollapsibleTrigger asChild>
            <Button
              type="button"
              variant="ghost"
              className="block h-auto w-full justify-start rounded-md p-0 text-left hover:bg-transparent"
              aria-controls={expandedId}
            >
              <div className="w-full">
                <div className="flex items-start justify-between gap-4">
                  <div className="min-w-0">
                    <CardTitle className="truncate">
                      {senderIdentity.name}
                    </CardTitle>
                    <p className="mt-1 truncate text-sm text-muted-foreground">
                      {senderIdentity.email}
                    </p>
                  </div>
                  <Badge variant="outline" className="shrink-0">
                    {expanded ? 'Hide' : 'Show'} · {emailCountLabel}
                  </Badge>
                </div>

                <div className="mt-4 flex flex-col gap-2">
                  <p className="text-sm leading-6 text-card-foreground">{subject}</p>
                  <p className="line-clamp-2 text-sm leading-6 text-muted-foreground">
                    {preview}
                  </p>
                </div>
              </div>
            </Button>
          </CollapsibleTrigger>
        </CardHeader>

        <CollapsibleContent>
          <CardContent id={expandedId} className="border-t pt-3">
            {emails.length === 0 ? (
              <p className="text-sm text-muted-foreground">
                Pending email details are unavailable right now.
              </p>
            ) : (
              <ul className="flex flex-col gap-3">
                {emails.map((email) => (
                  <li
                    key={email.email_id}
                    className="rounded-lg border bg-muted/30 px-3 py-2"
                  >
                    <div className="flex flex-col gap-1 sm:flex-row sm:items-start sm:justify-between sm:gap-4">
                      <p className="min-w-0 text-sm font-medium text-foreground">
                        {email.subject || 'No subject'}
                      </p>
                      <time
                        dateTime={email.received_at ?? undefined}
                        className="shrink-0 text-xs text-muted-foreground"
                      >
                        {formatDate(email.received_at)}
                      </time>
                    </div>
                    <p className="mt-2 line-clamp-2 text-sm leading-6 text-muted-foreground">
                      {email.preview || 'Preview unavailable.'}
                    </p>
                  </li>
                ))}
              </ul>
            )}
          </CardContent>
        </CollapsibleContent>
      </Collapsible>

      <CardFooter className="flex-wrap gap-2">
        <Button
          ref={approveButtonRef}
          type="button"
          aria-label={isPending ? 'Saving…' : 'Approve'}
          aria-haspopup="menu"
          aria-expanded={routingOpen}
          onClick={showRoutingDropdown}
          disabled={isPending}
          size="sm"
        >
          {isPending ? 'Saving…' : 'Yes'}
        </Button>
        <Button
          type="button"
          aria-label="Deny"
          onClick={deny}
          disabled={isPending}
          variant="outline"
          size="sm"
        >
          No
        </Button>
      </CardFooter>

      <ScreenerRoutingDropdown
        open={routingOpen}
        anchorRect={routingAnchor}
        onClose={() => setRoutingOpen(false)}
        onSelect={approve}
      />

      {decision.isError ? (
        <CardContent>
          <Alert variant="destructive">
            <AlertDescription>
              {actionErrorMessage(decision.error, 'Decision')}
            </AlertDescription>
          </Alert>
        </CardContent>
      ) : null}
    </Card>
  );
}

export function ScreenerPage({ client }: { client?: HailApiClient } = {}) {
  const query = useScreenerView(client);

  let list;
  if (query.isPending) {
    list = <LoadingState label="Loading pending senders" />;
  } else if (query.isError) {
    list = (
      <ErrorState
        message={viewErrorMessage(query.error, 'Screener')}
        onRetry={() => void query.refetch()}
      />
    );
  } else {
    list = (
      <ListView
        items={query.data.senders}
        renderItem={(sender) => <PendingSenderCard sender={sender} client={client} />}
        keyExtractor={(sender) => sender.sender}
        hasMore={false}
        isLoadingMore={false}
        onLoadMore={() => {}}
        emptyState={<EmptyState />}
      />
    );
  }

  return (
    <AppShell
      title="The Screener"
      description="New senders end up here. Decide if they get in."
      actions={
        <Button asChild variant="outline" size="sm">
          <Link to="/screened-out">
            <ShieldOff data-icon="inline-start" aria-hidden="true" />
            Screened Out
          </Link>
        </Button>
      }
      list={list}
    />
  );
}
