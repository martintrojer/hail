import { useMemo, useRef, useState } from 'react';
import type { DeniedSender, HailApiClient } from '../api/client';
import { useDeniedSenders, useUndoDenyMutation } from '../api/query';
import { ErrorState } from '../components/ErrorState';
import { LoadingState } from '../components/LoadingState';
import { ListView } from '../components/ListView';
import {
  ScreenerRoutingDropdown,
  type ScreenerRoutingDestination,
} from '../components/ScreenerRoutingDropdown';
import { AppShell } from '../layout/AppShell';
import { pillButtonClass } from '../lib/buttonStyles';
import { formatDate } from '../lib/dates';
import { actionErrorMessage, viewErrorMessage } from '../lib/errorMessages';

interface ScreenedOutPageProps {
  client?: HailApiClient;
}

function AllowButton({
  sender,
  client,
  label = 'Allow',
}: {
  sender: string;
  client?: HailApiClient;
  label?: string;
}) {
  const [routingOpen, setRoutingOpen] = useState(false);
  const [routingAnchor, setRoutingAnchor] = useState<DOMRect | null>(null);
  const buttonRef = useRef<HTMLButtonElement | null>(null);
  const undo = useUndoDenyMutation(client);

  function showRoutingDropdown() {
    if (buttonRef.current) {
      setRoutingAnchor(buttonRef.current.getBoundingClientRect());
    }
    setRoutingOpen(true);
  }

  function allow(destination: ScreenerRoutingDestination) {
    undo.mutate({ address: sender, classify_as: destination });
  }

  return (
    <>
      <button
        ref={buttonRef}
        type="button"
        aria-haspopup="menu"
        aria-expanded={routingOpen}
        onClick={showRoutingDropdown}
        disabled={undo.isPending}
        className={`${pillButtonClass('primary', 'md')} self-start sm:self-auto`}
      >
        {undo.isPending ? 'Allowing…' : label}
      </button>
      <ScreenerRoutingDropdown
        open={routingOpen}
        anchorRect={routingAnchor}
        onClose={() => setRoutingOpen(false)}
        onSelect={allow}
      />
      {undo.isError ? (
        <p role="alert" className="text-sm text-accent-red sm:col-span-2">
          {actionErrorMessage(undo.error, 'Decision')}
        </p>
      ) : null}
    </>
  );
}

function senderKey(sender: DeniedSender) {
  return sender.sender_address;
}

function ScreenedOutSenderCard({
  sender,
  client,
}: {
  sender: DeniedSender;
  client?: HailApiClient;
}) {
  return (
    <article className="rounded-lg border border-border-subtle bg-bg-surface p-4">
      <div className="grid gap-3 sm:grid-cols-[minmax(0,1fr)_auto] sm:items-center">
        <div className="min-w-0">
          <h3 className="truncate text-sm font-semibold text-ink-primary">
            {sender.sender_address}
          </h3>
          <p className="mt-1 text-xs text-ink-tertiary">
            Denied {formatDate(sender.denied_at)}
          </p>
          <p className="mt-2 line-clamp-2 text-sm text-ink-secondary">
            Individual screened-out email previews are not indexed here yet. Allowing
            this sender approves them and moves matching Trash/Screener mail to the
            selected destination.
          </p>
        </div>
        <AllowButton sender={sender.sender_address} client={client} />
      </div>
    </article>
  );
}

function BlockedSenderRow({
  sender,
  client,
}: {
  sender: DeniedSender;
  client?: HailApiClient;
}) {
  return (
    <div className="grid gap-3 rounded-lg border border-border-subtle bg-bg-surface px-4 py-3 sm:grid-cols-[minmax(0,1fr)_auto] sm:items-center">
      <div className="min-w-0">
        <p className="truncate text-sm font-semibold text-ink-primary">
          {sender.sender_address}
        </p>
        <p className="mt-1 text-xs text-ink-tertiary">
          Denied {formatDate(sender.denied_at)}
        </p>
      </div>
      <AllowButton sender={sender.sender_address} client={client} />
    </div>
  );
}

export function ScreenedOutPage({ client }: ScreenedOutPageProps) {
  const query = useDeniedSenders(client);
  const denied = useMemo(
    () => (query.isSuccess ? query.data.denied : []),
    [query.data?.denied, query.isSuccess],
  );

  let list;
  if (query.isPending) {
    list = <LoadingState label="Loading screened-out senders" />;
  } else if (query.isError) {
    list = (
      <ErrorState
        message={viewErrorMessage(query.error, 'Screened Out')}
        onRetry={() => void query.refetch()}
      />
    );
  } else {
    list = (
      <div className="space-y-8">
        <section aria-labelledby="screened-out-emails-heading">
          <div className="mb-4">
            <h2 id="screened-out-emails-heading" className="text-lg font-semibold text-ink-primary">
              Screened-out emails
            </h2>
            <p className="mt-1 text-sm text-ink-secondary">
              Mail from denied senders. Sender cards stand in for per-message
              previews until the denied-email view includes subjects and snippets.
            </p>
          </div>
          <ListView
            items={denied}
            renderItem={(sender) => (
              <ScreenedOutSenderCard sender={sender} client={client} />
            )}
            keyExtractor={senderKey}
            hasMore={false}
            isLoadingMore={false}
            onLoadMore={() => {}}
            emptyState={
              <p className="rounded-lg bg-bg-surface p-6 text-center text-sm text-ink-tertiary">
                No screened-out emails.
              </p>
            }
          />
        </section>

        <section aria-labelledby="blocked-senders-heading">
          <div className="mb-4">
            <h2 id="blocked-senders-heading" className="text-lg font-semibold text-ink-primary">
              Blocked senders
            </h2>
            <p className="mt-1 text-sm text-ink-secondary">
              Allow a sender and choose whether future and matching historical mail
              belongs in Imbox, Feed, or Paper Trail.
            </p>
          </div>
          <ListView
            items={denied}
            renderItem={(sender) => <BlockedSenderRow sender={sender} client={client} />}
            keyExtractor={senderKey}
            hasMore={false}
            isLoadingMore={false}
            onLoadMore={() => {}}
            emptyState={
              <p className="rounded-lg bg-bg-surface p-6 text-center text-sm text-ink-tertiary">
                No blocked senders.
              </p>
            }
          />
        </section>
      </div>
    );
  }

  return (
    <AppShell
      title="Screened Out"
      description="Review blocked senders and allow mistakes into the right place."
      list={list}
    />
  );
}
