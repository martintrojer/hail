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
import { Alert, AlertDescription } from '../components/ui/alert';
import { Badge } from '../components/ui/badge';
import { Button } from '../components/ui/button';
import {
  Card,
  CardAction,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from '../components/ui/card';
import { Empty, EmptyDescription, EmptyHeader, EmptyTitle } from '../components/ui/empty';
import { Tabs, TabsList, TabsTrigger } from '../components/ui/tabs';
import { AppShell } from '../layout/AppShell';
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
      <Button
        ref={buttonRef}
        type="button"
        aria-haspopup="menu"
        aria-expanded={routingOpen}
        onClick={showRoutingDropdown}
        disabled={undo.isPending}
        size="sm"
      >
        {undo.isPending ? 'Allowing…' : label}
      </Button>
      <ScreenerRoutingDropdown
        open={routingOpen}
        anchorRect={routingAnchor}
        onClose={() => setRoutingOpen(false)}
        onSelect={allow}
      />
      {undo.isError ? (
        <Alert variant="destructive" className="sm:col-span-2">
          <AlertDescription>
            {actionErrorMessage(undo.error, 'Decision')}
          </AlertDescription>
        </Alert>
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
    <Card size="sm">
      <CardHeader>
        <CardTitle className="truncate">{sender.sender_address}</CardTitle>
        <CardDescription>Denied {formatDate(sender.denied_at)}</CardDescription>
        <CardAction>
          <AllowButton sender={sender.sender_address} client={client} />
        </CardAction>
      </CardHeader>
      <CardContent>
        <p className="line-clamp-2 text-sm text-muted-foreground">
          Individual screened-out email previews are not indexed here yet. Allowing
          this sender approves them and moves matching Trash/Screener mail to the
          selected destination.
        </p>
      </CardContent>
    </Card>
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
    <Card size="sm">
      <CardHeader>
        <CardTitle className="truncate">{sender.sender_address}</CardTitle>
        <CardDescription>Denied {formatDate(sender.denied_at)}</CardDescription>
        <CardAction>
          <AllowButton sender={sender.sender_address} client={client} />
        </CardAction>
      </CardHeader>
    </Card>
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
    <Tabs
      value={active}
      onValueChange={(value) => {
        if (value === 'emails' || value === 'senders') {
          onChange(value);
        }
      }}
      className="mb-4"
    >
      <TabsList>
        {tabs.map((tab) => (
          <TabsTrigger
            key={tab.id}
            value={tab.id}
            onClick={() => onChange(tab.id)}
          >
            {tab.label}
            {tab.count > 0 ? <Badge variant="secondary">{tab.count}</Badge> : null}
          </TabsTrigger>
        ))}
      </TabsList>
    </Tabs>
  );
}

function EmptyScreenedOutState({ message }: { message: string }) {
  return (
    <Empty className="min-h-[220px]">
      <EmptyHeader>
        <EmptyTitle>{message}</EmptyTitle>
        <EmptyDescription>
          Denied sender decisions will appear here if you change your mind.
        </EmptyDescription>
      </EmptyHeader>
    </Empty>
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
            emptyState={<EmptyScreenedOutState message={emptyMessage} />}
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
            emptyState={<EmptyScreenedOutState message={emptyMessage} />}
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
