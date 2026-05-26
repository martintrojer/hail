import { useMemo, useRef, useState } from 'react';
import type {
  HailApiClient,
  ScreenerAllowedSender,
  ScreenerClassification,
} from '../api/client';
import {
  useScreenerAllowedView,
  useScreenerDecisionMutation,
} from '../api/query';
import { ErrorState } from '../components/ErrorState';
import { LoadingState } from '../components/LoadingState';
import { ListView } from '../components/ListView';
import {
  routingDestinationLabels,
  ScreenerRoutingDropdown,
  type ScreenerRoutingDestination,
} from '../components/ScreenerRoutingDropdown';
import { AppShell } from '../layout/AppShell';
import { pillButtonClass } from '../lib/buttonStyles';
import { formatDate } from '../lib/dates';
import { actionErrorMessage, viewErrorMessage } from '../lib/errorMessages';

interface ScreenerSpeakeasyPageProps {
  client?: HailApiClient;
}

function classificationLabel(classification: ScreenerClassification) {
  return routingDestinationLabels[classification];
}

function EmptyState() {
  return (
    <div className="rounded-2xl bg-bg-surface px-6 py-10 text-center">
      <p className="text-lg font-semibold text-ink-primary">
        No approved senders yet.
      </p>
      <p className="mx-auto mt-2 max-w-md text-sm leading-6 text-ink-secondary">
        Let someone in from The Screener and they’ll appear here with the place
        their future mail is routed.
      </p>
    </div>
  );
}

function SpeakeasyIntro({ count }: { count: number }) {
  const senderLabel = count === 1 ? 'sender' : 'senders';

  return (
    <section className="mb-5 rounded-2xl border border-border-subtle bg-bg-surface p-5">
      <p className="text-xs font-semibold uppercase tracking-[0.18em] text-ink-tertiary">
        Screener Speakeasy
      </p>
      <h2 className="mt-2 text-xl font-semibold text-ink-primary">
        Approved senders bypass the rope.
      </h2>
      <p className="mt-2 max-w-2xl text-sm leading-6 text-ink-secondary">
        This is your pass list: senders you’ve allowed through The Screener and
        the destination hail should use for their future mail. Sender addresses
        and routing values come from the API rule table.
      </p>
      <p className="mt-4 text-sm font-medium text-ink-primary">
        {count} approved {senderLabel}
      </p>
    </section>
  );
}

function AllowedSenderRow({
  sender,
  client,
}: {
  sender: ScreenerAllowedSender;
  client?: HailApiClient;
}) {
  const [routingOpen, setRoutingOpen] = useState(false);
  const [routingAnchor, setRoutingAnchor] = useState<DOMRect | null>(null);
  const buttonRef = useRef<HTMLButtonElement | null>(null);
  const decision = useScreenerDecisionMutation(client);

  function showRoutingDropdown() {
    if (buttonRef.current) {
      setRoutingAnchor(buttonRef.current.getBoundingClientRect());
    }
    setRoutingOpen(true);
  }

  function changeRoute(destination: ScreenerRoutingDestination) {
    decision.mutate({
      sender: sender.sender_address,
      decision: 'approve',
      classify_as: destination,
      apply_to_history: true,
    });
  }

  return (
    <article className="rounded-lg bg-bg-surface px-4 py-4">
      <div className="grid gap-3 sm:grid-cols-[minmax(0,1fr)_auto] sm:items-center">
        <div className="min-w-0">
          <h3 className="truncate text-sm font-semibold text-ink-primary">
            {sender.sender_address}
          </h3>
          <div className="mt-2 flex flex-wrap items-center gap-2 text-xs text-ink-tertiary">
            <span className="rounded-full border border-border-menu px-2.5 py-1 font-semibold text-ink-secondary">
              Routed to {classificationLabel(sender.classify_as)}
            </span>
            {sender.decided_at ? (
              <span>Approved {formatDate(sender.decided_at)}</span>
            ) : (
              <span>Approved sender</span>
            )}
          </div>
        </div>
        <button
          ref={buttonRef}
          type="button"
          aria-haspopup="menu"
          aria-expanded={routingOpen}
          onClick={showRoutingDropdown}
          disabled={decision.isPending}
          className={`${pillButtonClass('outline', 'md')} self-start sm:self-auto`}
        >
          {decision.isPending ? 'Saving…' : 'Change route'}
        </button>
      </div>
      <ScreenerRoutingDropdown
        open={routingOpen}
        anchorRect={routingAnchor}
        onClose={() => setRoutingOpen(false)}
        onSelect={changeRoute}
        value={sender.classify_as}
      />
      {decision.isError ? (
        <p role="alert" className="mt-3 text-sm text-accent-red">
          {actionErrorMessage(decision.error, 'Decision')}
        </p>
      ) : null}
    </article>
  );
}

function allowedSenderKey(sender: ScreenerAllowedSender) {
  return sender.sender_address;
}

export function ScreenerSpeakeasyPage({ client }: ScreenerSpeakeasyPageProps) {
  const query = useScreenerAllowedView(client);
  const allowed = useMemo(
    () => (query.isSuccess ? query.data.allowed : []),
    [query.data?.allowed, query.isSuccess],
  );

  let body;
  if (query.isPending) {
    body = <LoadingState label="Loading approved senders" />;
  } else if (query.isError) {
    body = (
      <ErrorState
        message={viewErrorMessage(query.error, 'Screener Speakeasy')}
        onRetry={() => void query.refetch()}
      />
    );
  } else {
    body = (
      <>
        <SpeakeasyIntro count={allowed.length} />
        <ListView
          items={allowed}
          renderItem={(sender) => (
            <AllowedSenderRow sender={sender} client={client} />
          )}
          keyExtractor={allowedSenderKey}
          hasMore={false}
          isLoadingMore={false}
          onLoadMore={() => {}}
          emptyState={<EmptyState />}
        />
      </>
    );
  }

  return (
    <AppShell
      title="Speakeasy"
      description="Allowed senders and where their mail goes."
      list={body}
    />
  );
}
