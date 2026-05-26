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
    <article className="rounded-lg bg-bg-surface p-4">
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
    <div className="grid gap-3 rounded-lg bg-bg-surface px-4 py-3 sm:grid-cols-[minmax(0,1fr)_auto] sm:items-center">
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

type ScreenedOutTab = 'emails' | 'senders';

function TabBar({
  active,
  onChange,
  counts,
}: {
  active: ScreenedOutTab;
  onChange: (tab: ScreenedOutTab) => void;
  counts: { emails: number; senders: number };
}) {
  const tabs: Array<{ id: ScreenedOutTab; label: string; count: number }> = [
    { id: 'emails', label: 'Screened Emails', count: counts.emails },
    { id: 'senders', label: 'Blocked Senders', count: counts.senders },
  ];

  return (
    <div className="mb-4 flex gap-1 rounded-lg bg-bg-canvas p-1" role="tablist">
      {tabs.map((tab) => (
        <button
          key={tab.id}
          role="tab"
          type="button"
          aria-selected={active === tab.id}
          onClick={() => onChange(tab.id)}
          className={`flex-1 rounded-md px-3 py-1.5 text-sm font-medium transition-colors ${
            active === tab.id
              ? 'bg-bg-surface text-ink-primary shadow-sm'
              : 'text-ink-tertiary hover:text-ink-secondary'
          }`}
        >
          {tab.label}
          {tab.count > 0 ? (
            <span className="ml-1.5 text-xs text-ink-tertiary">({tab.count})</span>
          ) : null}
        </button>
      ))}
    </div>
  );
}

export function ScreenedOutPage({ client }: ScreenedOutPageProps) {
  const [activeTab, setActiveTab] = useState<ScreenedOutTab>('emails');
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
    const emptyMessage =
      activeTab === 'emails'
        ? 'No screened-out emails.'
        : 'No blocked senders.';

    list = (
      <div>
        <TabBar
          active={activeTab}
          onChange={setActiveTab}
          counts={{ emails: denied.length, senders: denied.length }}
        />
        {activeTab === 'emails' ? (
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
                {emptyMessage}
              </p>
            }
          />
        ) : (
          <ListView
            items={denied}
            renderItem={(sender) => (
              <BlockedSenderRow sender={sender} client={client} />
            )}
            keyExtractor={senderKey}
            hasMore={false}
            isLoadingMore={false}
            onLoadMore={() => {}}
            emptyState={
              <p className="rounded-lg bg-bg-surface p-6 text-center text-sm text-ink-tertiary">
                {emptyMessage}
              </p>
            }
          />
        )}
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
