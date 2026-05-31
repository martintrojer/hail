import { useEffect, useMemo } from 'react';
import type { HailApiClient, ProviderSyncStatus } from '../api/client';
import {
  useConnectGmailMutation,
  useDisconnectProviderAccountMutation,
  useProviderSyncStatuses,
  useReimportProviderAccountMutation,
  useStopProviderSyncMutation,
  useTriggerProviderSyncMutation,
} from '../api/query';
import { StateCard } from '../components/StateCard';
import { Alert, AlertDescription } from '../components/ui/alert';
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
  AlertDialogTrigger,
} from '../components/ui/alert-dialog';
import { Badge } from '../components/ui/badge';
import { Button } from '../components/ui/button';
import { Card, CardAction, CardContent, CardDescription, CardHeader, CardTitle } from '../components/ui/card';
import { BadgeCheck, Loader2 } from '../components/icons';
import { AppShell } from '../layout/AppShell';
import { actionErrorMessage } from '../lib/errorMessages';

interface ProviderAccountsPageProps {
  client?: HailApiClient;
  initialAccount?: ProviderSyncStatus | null;
  location?: Pick<Location, 'assign'> & { search?: string };
  confirmDisconnect?: (message: string) => boolean;
}

type ProviderSyncStatusValue =
  | 'disabled'
  | 'initial_sync'
  | 'active'
  | 'error'
  | 'paused'
  | 'revoked'
  | 'disconnected'
  | (string & {});

function statusLabel(status: ProviderSyncStatusValue, recovered = false) {
  if (status === 'error' && recovered) return 'Connected (recovered)';
  switch (status) {
    case 'disabled': return 'Disabled';
    case 'initial_sync': return 'Initial import running';
    case 'active': return 'Connected';
    case 'error': return 'Needs attention';
    case 'paused': return 'Paused';
    case 'revoked': return 'Access revoked';
    case 'disconnected': return 'Disconnected';
    default: return status || 'Unknown';
  }
}

function canTriggerSync(status: ProviderSyncStatusValue) {
  return !['disabled', 'disconnected', 'revoked'].includes(status);
}

function canReimport(status: ProviderSyncStatusValue) {
  return !['disabled', 'disconnected', 'revoked'].includes(status);
}

function statusVariant(status: ProviderSyncStatusValue): 'secondary' | 'destructive' | 'outline' {
  switch (status) {
    case 'active':
    case 'initial_sync':
      return 'secondary';
    case 'error':
    case 'revoked':
      return 'destructive';
    default:
      return 'outline';
  }
}

function formatDateTime(value: string | null | undefined) {
  if (!value) return 'Not available yet';
  return new Intl.DateTimeFormat(undefined, { dateStyle: 'medium', timeStyle: 'short' }).format(new Date(value));
}

function formatBackoff(value: number | null | undefined) {
  if (value === null || value === undefined) return 'No retry backoff';
  if (value < 60) return `${value} seconds`;
  const minutes = Math.round(value / 60);
  if (minutes < 60) return `${minutes} ${minutes === 1 ? 'minute' : 'minutes'}`;
  const hours = Math.round(minutes / 60);
  return `${hours} ${hours === 1 ? 'hour' : 'hours'}`;
}

function isRecovered(status: ProviderSyncStatus) {
  return Boolean(
    status.last_sync_succeeded_at &&
    (!status.last_error_event || new Date(status.last_sync_succeeded_at) > new Date(status.last_error_event.created_at)),
  );
}

function isServerSyncing(status: ProviderSyncStatus) {
  if (status.sync_status === 'paused') return false;
  return status.sync_status === 'initial_sync' || Boolean(
    status.last_sync_attempted_at &&
    (!status.last_sync_succeeded_at || new Date(status.last_sync_attempted_at) > new Date(status.last_sync_succeeded_at)),
  );
}

function isHealthyBadge(status: ProviderSyncStatus, recovered: boolean) {
  return (status.sync_status === 'active' && (!status.last_error_event || recovered)) ||
    (status.sync_status === 'error' && recovered);
}

function StatusBadge({ status }: { status: ProviderSyncStatus }) {
  const recovered = isRecovered(status);
  const healthy = isHealthyBadge(status, recovered);

  return (
    <Badge
      variant={healthy ? 'secondary' : statusVariant(status.sync_status)}
      className={healthy ? 'mt-3 border-emerald-500/40 bg-emerald-50 text-emerald-900 dark:border-emerald-400/40 dark:bg-emerald-950/40 dark:text-emerald-100 [&>svg]:text-emerald-600 dark:[&>svg]:text-emerald-400' : 'mt-3'}
    >
      {healthy ? <BadgeCheck aria-hidden="true" /> : null}
      {statusLabel(status.sync_status, recovered)}
    </Badge>
  );
}

function failureText(status: ProviderSyncStatus) {
  if (isRecovered(status)) return 'No active failure — last sync succeeded';
  const className = status.last_error_event?.safe_error_class || status.last_error_class;
  const message = status.last_error_event?.safe_error_message;
  if (className && message) return `${className}: ${message}`;
  return className || message || 'No recent failure recorded';
}

function callbackErrorMessage(error: string) {
  switch (error) {
    case 'oauth_denied': return 'Gmail connection was cancelled before hail received access.';
    case 'missing_state':
    case 'missing_code':
    case 'invalid_oauth_state': return 'Gmail connection could not be verified. Please try connecting again.';
    case 'oauth_exchange_failed': return 'Gmail connection failed while exchanging authorization with Google. Please try again.';
    case 'callback_failed': return 'Gmail connected at Google, but hail could not finish saving the account. Please try again.';
    default: return 'Gmail connection did not complete. Please try again.';
  }
}

function providerCallbackNotice(search: string | undefined) {
  const params = new URLSearchParams(search ?? '');
  if (params.get('connected') === 'gmail') {
    return { kind: 'connected' as const, message: 'Gmail connected. Hail is refreshing import status now.' };
  }
  const error = params.get('error');
  if (error) {
    return { kind: 'error' as const, message: callbackErrorMessage(error) };
  }
  return null;
}

function DetailTile({ label, value }: { label: string; value: string }) {
  return (
    <Card size="sm" className="bg-muted/40 shadow-none">
      <CardContent>
        <dt className="text-xs font-semibold uppercase tracking-wide text-muted-foreground">{label}</dt>
        <dd className="mt-1 break-words text-sm">{value}</dd>
      </CardContent>
    </Card>
  );
}

function FailureCard({ status, recovered }: { status: ProviderSyncStatus; recovered: boolean }) {
  const card = (
    <Card size="sm" className="mt-4 bg-muted/40 shadow-none">
      <CardContent>
        <p className="text-xs font-semibold uppercase tracking-wide text-muted-foreground">Last failure</p>
        <p className="mt-1 text-sm">{failureText(status)}</p>
        {status.last_error_event ? <p className="mt-1 text-xs text-muted-foreground">Recorded {formatDateTime(status.last_error_event.created_at)} during {status.last_error_event.event_type}.</p> : null}
      </CardContent>
    </Card>
  );

  if (!recovered) return card;

  return (
    <details className="mt-4">
      <summary className="cursor-pointer text-sm font-medium text-muted-foreground">
        Show last failure (recovered {formatDateTime(status.last_sync_succeeded_at)})
      </summary>
      {card}
    </details>
  );
}

function ProviderSyncStatusCard({ status, syncing, reimporting, stopping, syncError, reimportError, stopError, onSync, onReimport, onStop }: {
  status: ProviderSyncStatus;
  syncing: boolean;
  reimporting: boolean;
  stopping: boolean;
  syncError: Error | null;
  reimportError: Error | null;
  stopError: Error | null;
  onSync: () => void;
  onReimport: () => void;
  onStop: () => void;
}) {
  const recovered = isRecovered(status);
  const serverSyncing = isServerSyncing(status);
  const syncInProgress = syncing || serverSyncing;
  const initialSync = status.sync_status === 'initial_sync';
  const paused = status.sync_status === 'paused';
  const showStopImport = serverSyncing || initialSync;
  const actionsDisabled = syncInProgress || reimporting || stopping;
  const syncDisabled = actionsDisabled || (!paused && !canTriggerSync(status.sync_status));
  const reimportDisabled = actionsDisabled;
  const title = initialSync ? 'Re-importing from Gmail…' : (status.display_email || status.provider_email);
  return (
    <section>
      <Card>
        <CardHeader>
          <div>
            <CardDescription className="text-xs font-semibold uppercase tracking-wide">Gmail import health</CardDescription>
            <CardTitle role="heading" aria-level={2}>{title}</CardTitle>
            <div className="flex flex-wrap items-center gap-3">
              <StatusBadge status={status} />
              {syncInProgress ? (
                <span className="mt-3 inline-flex items-center gap-1 text-sm font-medium text-muted-foreground" role="status">
                  <Loader2 className="size-4 animate-spin" aria-hidden="true" />
                  {initialSync ? 'Re-importing from Gmail…' : 'Syncing Gmail…'}
                </span>
              ) : null}
            </div>
          </div>
          <CardAction className="flex flex-col gap-2 sm:flex-row">
            {showStopImport ? (
              <AlertDialog>
                <AlertDialogTrigger asChild>
                  <Button type="button" variant="destructive" disabled={stopping}>
                    {stopping ? 'Pausing…' : 'Stop import'}
                  </Button>
                </AlertDialogTrigger>
                <AlertDialogContent>
                  <AlertDialogHeader>
                    <AlertDialogTitle>Stop Gmail import?</AlertDialogTitle>
                    <AlertDialogDescription>
                      The current sync will pause after the in-flight batch. Already-imported messages stay. Resume with Sync now or Re-import.
                    </AlertDialogDescription>
                  </AlertDialogHeader>
                  <AlertDialogFooter>
                    <AlertDialogCancel disabled={stopping}>Cancel</AlertDialogCancel>
                    <AlertDialogAction disabled={stopping} onClick={onStop}>Stop import</AlertDialogAction>
                  </AlertDialogFooter>
                </AlertDialogContent>
              </AlertDialog>
            ) : null}
            <Button type="button" onClick={onSync} disabled={syncDisabled}>
              {syncing ? 'Requesting sync…' : 'Sync now'}
            </Button>
            {canReimport(status.sync_status) ? (
              <AlertDialog>
                <AlertDialogTrigger asChild>
                  <Button type="button" variant="outline" disabled={reimportDisabled}>
                    {reimporting ? 'Requesting re-import…' : 'Re-import from Gmail'}
                  </Button>
                </AlertDialogTrigger>
                <AlertDialogContent>
                  <AlertDialogHeader>
                    <AlertDialogTitle>Re-import from Gmail?</AlertDialogTitle>
                    <AlertDialogDescription>
                      Hail will re-fetch all Gmail messages from scratch. This may take a long time and hit Gmail API quota. Already-imported local mail is kept and will be deduped by content hash.
                    </AlertDialogDescription>
                  </AlertDialogHeader>
                  <AlertDialogFooter>
                    <AlertDialogCancel disabled={reimporting}>Cancel</AlertDialogCancel>
                    <AlertDialogAction disabled={reimporting} onClick={onReimport}>Re-import</AlertDialogAction>
                  </AlertDialogFooter>
                </AlertDialogContent>
              </AlertDialog>
            ) : null}
          </CardAction>
        </CardHeader>
        <CardContent>
          {paused ? <p className="mb-4 text-sm font-medium text-muted-foreground">Paused — click Sync now to resume</p> : null}
          <dl className="grid gap-3 sm:grid-cols-2 lg:grid-cols-3">
            <DetailTile label="Last sync attempt" value={formatDateTime(status.last_sync_attempted_at)} />
            <DetailTile label="Last successful sync" value={formatDateTime(status.last_sync_succeeded_at)} />
            <DetailTile label="Next retry" value={formatDateTime(status.next_sync_after)} />
            <DetailTile label="Retry backoff" value={formatBackoff(status.sync_backoff_secs)} />
            <DetailTile label="Gmail history cursor" value={status.last_profile_history_id || 'Not captured yet'} />
            <DetailTile label="Profile synced" value={formatDateTime(status.profile_synced_at)} />
          </dl>

          <FailureCard status={status} recovered={recovered} />

          {syncError ? <Alert variant="destructive" className="mt-4"><AlertDescription>{actionErrorMessage(syncError, 'Sync Gmail now')}</AlertDescription></Alert> : null}
          {reimportError ? <Alert variant="destructive" className="mt-4"><AlertDescription>{actionErrorMessage(reimportError, 'Re-import from Gmail')}</AlertDescription></Alert> : null}
          {stopError ? <Alert variant="destructive" className="mt-4"><AlertDescription>{actionErrorMessage(stopError, 'Stop Gmail import')}</AlertDescription></Alert> : null}
        </CardContent>
      </Card>
    </section>
  );
}

function ProviderAccountCard({ account, disconnecting, disconnectError, onDisconnect }: {
  account: ProviderSyncStatus;
  disconnecting: boolean;
  disconnectError: Error | null;
  onDisconnect: () => void;
}) {
  const connected = account.sync_status !== 'disconnected';

  return (
    <Card>
      <CardHeader>
        <div>
          <CardDescription className="text-xs font-semibold uppercase tracking-wide">Gmail account</CardDescription>
          <CardTitle>{account.display_email || account.provider_email}</CardTitle>
          <StatusBadge status={account} />
        </div>
        <CardAction><Button type="button" variant="destructive" onClick={onDisconnect} disabled={disconnecting || !connected}>
          {disconnecting ? 'Disconnecting…' : connected ? 'Disconnect' : 'Disconnected'}
        </Button></CardAction>
      </CardHeader>
      <CardContent>
      <dl className="grid gap-3 sm:grid-cols-2">
        <DetailTile label="Provider id" value={account.provider_account_id} />
        <DetailTile label="Gmail history cursor" value={account.last_profile_history_id || 'Not captured yet'} />
        <DetailTile label="Profile synced" value={formatDateTime(account.profile_synced_at)} />
      </dl>

      {disconnectError ? <Alert variant="destructive" className="mt-4"><AlertDescription>{actionErrorMessage(disconnectError, 'Disconnect Gmail')}</AlertDescription></Alert> : null}
      </CardContent>
    </Card>
  );
}

export function ProviderAccountsPage({ client, location = window.location, confirmDisconnect = window.confirm }: ProviderAccountsPageProps) {
  const callbackNotice = useMemo(() => providerCallbackNotice(location.search), [location.search]);
  const connectGmail = useConnectGmailMutation(client, { onSuccess: (data) => location.assign(data.authorization_url) });
  const disconnectProviderAccount = useDisconnectProviderAccountMutation(client);
  const syncStatuses = useProviderSyncStatuses(client);
  const triggerSync = useTriggerProviderSyncMutation(client);
  const reimportProviderAccount = useReimportProviderAccountMutation(client);
  const stopProviderSync = useStopProviderSyncMutation(client);
  const { refetch: refetchSyncStatuses } = syncStatuses;

  useEffect(() => {
    if (callbackNotice) {
      void refetchSyncStatuses();
    }
  }, [callbackNotice, refetchSyncStatuses]);

  const gmailStatuses = syncStatuses.data?.accounts.filter((status) => status.provider_kind === 'gmail') ?? [];

  function onConnect() {
    connectGmail.mutate();
  }

  function onDisconnect(status: ProviderSyncStatus) {
    const confirmed = confirmDisconnect(`Disconnect ${status.provider_email} from Gmail import? Hail will keep already-imported local mail, but new Gmail mail will stop importing.`);
    if (confirmed) disconnectProviderAccount.mutate(status.id);
  }

  const list = (
    <div className="flex flex-col gap-5">
      {callbackNotice ? (
        callbackNotice.kind === 'connected' ? (
          <Alert
            role="status"
            className="border-emerald-500/40 bg-emerald-50 text-emerald-900 dark:border-emerald-400/40 dark:bg-emerald-950/40 dark:text-emerald-100 [&>svg]:text-emerald-600 dark:[&>svg]:text-emerald-400"
          >
            <BadgeCheck className="size-5" aria-hidden="true" />
            <AlertDescription className="text-sm font-medium text-emerald-900 dark:text-emerald-100">
              {callbackNotice.message}
            </AlertDescription>
          </Alert>
        ) : (
          <Alert variant="destructive">
            <AlertDescription>{callbackNotice.message}</AlertDescription>
          </Alert>
        )
      ) : null}

      <Card>
        <CardHeader>
          <div>
            <CardDescription className="text-xs font-semibold uppercase tracking-wide">Provider import mode</CardDescription>
            <CardTitle role="heading" aria-level={2}>Connect Gmail</CardTitle>
            <CardDescription className="max-w-2xl leading-6">Gmail remains your public mailbox and spam filter. Hail imports a local Stalwart copy and the normal hail UI reads that local copy. During v1.2 import, hail actions do not archive, delete, mark read, or relabel Gmail mail.</CardDescription>
          </div>
          <CardAction>
            <Button type="button" onClick={onConnect} disabled={connectGmail.isPending}>
              {connectGmail.isPending ? 'Opening Google…' : gmailStatuses.length > 0 ? 'Reconnect Gmail' : 'Connect Gmail'}
            </Button>
          </CardAction>
        </CardHeader>

        <CardContent className="grid gap-3 sm:grid-cols-3">
          <Card size="sm" className="bg-muted/40 shadow-none"><CardHeader><CardTitle>Read-only scope</CardTitle><CardDescription>Hail requests Gmail read-only access for one-way import.</CardDescription></CardHeader></Card>
          <Card size="sm" className="bg-muted/40 shadow-none"><CardHeader><CardTitle>Tokens stay server-side</CardTitle><CardDescription>The browser only receives an authorization URL, never OAuth tokens.</CardDescription></CardHeader></Card>
          <Card size="sm" className="bg-muted/40 shadow-none"><CardHeader><CardTitle>Local truth</CardTitle><CardDescription>Imported messages are viewed and routed from Stalwart, not Gmail labels.</CardDescription></CardHeader></Card>
        </CardContent>

        {connectGmail.error ? <CardContent><Alert variant="destructive"><AlertDescription>{actionErrorMessage(connectGmail.error, 'Connect Gmail')}</AlertDescription></Alert></CardContent> : null}
      </Card>

      {syncStatuses.isPending ? (
        <StateCard title="Checking Gmail import status" body="Loading Gmail sync health from hail-api." />
      ) : syncStatuses.isError ? (
        <Alert variant="destructive"><AlertDescription>{actionErrorMessage(syncStatuses.error, 'Load Gmail import status')}</AlertDescription></Alert>
      ) : gmailStatuses.length > 0 ? (
        <>
          {gmailStatuses.map((status) => (
            <div key={status.id} className="flex flex-col gap-5">
              <ProviderSyncStatusCard status={status} syncing={triggerSync.isPending && triggerSync.variables === status.id} reimporting={reimportProviderAccount.isPending && reimportProviderAccount.variables === status.id} stopping={stopProviderSync.isPending && stopProviderSync.variables === status.id} syncError={triggerSync.variables === status.id ? triggerSync.error : null} reimportError={reimportProviderAccount.variables === status.id ? reimportProviderAccount.error : null} stopError={stopProviderSync.variables === status.id ? stopProviderSync.error : null} onSync={() => triggerSync.mutate(status.id)} onReimport={() => reimportProviderAccount.mutate(status.id)} onStop={() => stopProviderSync.mutate(status.id)} />
              <ProviderAccountCard account={status} disconnecting={disconnectProviderAccount.isPending && disconnectProviderAccount.variables === status.id} disconnectError={disconnectProviderAccount.variables === status.id ? disconnectProviderAccount.error : null} onDisconnect={() => onDisconnect(status)} />
            </div>
          ))}
        </>
      ) : (
        <StateCard title="No Gmail account connected" body="Connect Gmail to start the server-side OAuth flow. Once Google redirects back, import status will appear here." />
      )}
    </div>
  );

  return <AppShell title="Provider Accounts" description="Connect Gmail for one-way provider import into hail." list={list} wide />;
}
