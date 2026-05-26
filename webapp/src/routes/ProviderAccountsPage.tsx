import { useEffect, useMemo, useState } from 'react';
import type { HailApiClient, ProviderAccount, ProviderSyncStatus } from '../api/client';
import {
  useConnectGmailMutation,
  useDisconnectProviderAccountMutation,
  useProviderSyncStatuses,
  useTriggerProviderSyncMutation,
} from '../api/query';
import { StateCard } from '../components/StateCard';
import { AppShell } from '../layout/AppShell';
import { actionErrorMessage } from '../lib/errorMessages';

interface ProviderAccountsPageProps {
  client?: HailApiClient;
  initialAccount?: ProviderAccount | null;
  location?: Pick<Location, 'assign'> & { search?: string };
  confirmDisconnect?: (message: string) => boolean;
}

const GMAIL_READONLY_SCOPE = 'https://www.googleapis.com/auth/gmail.readonly';

type ProviderSyncStatusValue =
  | 'disabled'
  | 'initial_sync'
  | 'active'
  | 'error'
  | 'revoked'
  | 'disconnected'
  | (string & {});

function statusLabel(status: ProviderSyncStatusValue) {
  switch (status) {
    case 'disabled': return 'Disabled';
    case 'initial_sync': return 'Initial import running';
    case 'active': return 'Connected';
    case 'error': return 'Needs attention';
    case 'revoked': return 'Access revoked';
    case 'disconnected': return 'Disconnected';
    default: return status || 'Unknown';
  }
}

function healthTone(status: ProviderSyncStatusValue) {
  switch (status) {
    case 'active':
    case 'initial_sync':
      return 'bg-accent-green';
    case 'error':
    case 'revoked':
      return 'bg-accent-red';
    case 'disabled':
      return 'bg-accent-yellow';
    case 'disconnected':
      return 'bg-ink-tertiary';
    default:
      return 'bg-ink-tertiary';
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

function scopeLabel(scope: string) {
  return scope === GMAIL_READONLY_SCOPE ? 'Gmail read-only import' : scope;
}

function failureText(status: ProviderSyncStatus) {
  const className = status.last_error_class || status.last_error_event?.safe_error_class;
  const message = status.last_error_message || status.last_error_event?.safe_error_message;
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
    <div className="rounded-lg border border-hairline bg-page p-3">
      <dt className="text-xs font-semibold uppercase tracking-wide text-ink-secondary">{label}</dt>
      <dd className="mt-1 break-words text-sm text-ink-primary">{value}</dd>
    </div>
  );
}

function ProviderSyncStatusCard({ status, syncing, syncError, onSync }: {
  status: ProviderSyncStatus;
  syncing: boolean;
  syncError: Error | null;
  onSync: () => void;
}) {
  return (
    <section className="rounded-2xl border border-hairline bg-surface p-5 shadow-sm">
      <div className="flex flex-col gap-4 sm:flex-row sm:items-start sm:justify-between">
        <div>
          <p className="text-xs font-semibold uppercase tracking-wide text-ink-secondary">Gmail import health</p>
          <h2 className="mt-1 text-xl font-semibold text-ink-primary">{status.display_email || status.provider_email}</h2>
          <div className="mt-3 inline-flex items-center gap-2 rounded-full border border-hairline bg-page px-3 py-1 text-sm font-medium text-ink-primary">
            <span aria-hidden="true" className={`h-2.5 w-2.5 rounded-full ${healthTone(status.sync_status)}`} />
            {statusLabel(status.sync_status)}
          </div>
        </div>
        <button type="button" onClick={onSync} disabled={syncing || status.sync_status === 'disconnected'} className="rounded-full bg-accent-blue px-5 py-2.5 text-sm font-semibold text-white transition hover:bg-accent-blue-hover disabled:cursor-not-allowed disabled:opacity-60">
          {syncing ? 'Requesting sync…' : 'Sync now'}
        </button>
      </div>

      <dl className="mt-5 grid gap-3 sm:grid-cols-2 lg:grid-cols-3">
        <DetailTile label="Last sync attempt" value={formatDateTime(status.last_sync_attempted_at)} />
        <DetailTile label="Last successful sync" value={formatDateTime(status.last_sync_succeeded_at)} />
        <DetailTile label="Next retry" value={formatDateTime(status.next_sync_after)} />
        <DetailTile label="Retry backoff" value={formatBackoff(status.sync_backoff_secs)} />
        <DetailTile label="Gmail history cursor" value={status.last_profile_history_id || 'Not captured yet'} />
        <DetailTile label="Profile synced" value={formatDateTime(status.profile_synced_at)} />
      </dl>

      <div className="mt-4 rounded-lg border border-hairline bg-page p-3">
        <p className="text-xs font-semibold uppercase tracking-wide text-ink-secondary">Last failure</p>
        <p className="mt-1 text-sm text-ink-primary">{failureText(status)}</p>
        {status.last_error_event ? <p className="mt-1 text-xs text-ink-secondary">Recorded {formatDateTime(status.last_error_event.created_at)} during {status.last_error_event.event_type}.</p> : null}
      </div>

      {syncError ? <p role="alert" className="mt-4 rounded-lg border border-accent-red/30 bg-accent-red/10 px-3 py-2 text-sm text-accent-red">{actionErrorMessage(syncError, 'Sync Gmail now')}</p> : null}
    </section>
  );
}

function ProviderAccountCard({ account, disconnecting, disconnectError, onDisconnect }: {
  account: ProviderAccount;
  disconnecting: boolean;
  disconnectError: Error | null;
  onDisconnect: () => void;
}) {
  const connected = account.sync_status !== 'disconnected';

  return (
    <section className="rounded-2xl border border-hairline bg-surface p-5 shadow-sm">
      <div className="flex flex-col gap-4 sm:flex-row sm:items-start sm:justify-between">
        <div>
          <p className="text-xs font-semibold uppercase tracking-wide text-ink-secondary">Gmail account</p>
          <h2 className="mt-1 text-xl font-semibold text-ink-primary">{account.display_email || account.provider_email}</h2>
          <div className="mt-3 inline-flex items-center gap-2 rounded-full border border-hairline bg-page px-3 py-1 text-sm font-medium text-ink-primary">
            <span aria-hidden="true" className={`h-2.5 w-2.5 rounded-full ${healthTone(account.sync_status)}`} />
            {statusLabel(account.sync_status)}
          </div>
        </div>
        <button type="button" onClick={onDisconnect} disabled={disconnecting || !connected} className="rounded-full border border-accent-red/40 px-4 py-2 text-sm font-semibold text-accent-red transition hover:bg-accent-red/10 disabled:cursor-not-allowed disabled:opacity-60">
          {disconnecting ? 'Disconnecting…' : connected ? 'Disconnect' : 'Disconnected'}
        </button>
      </div>

      <dl className="mt-5 grid gap-3 sm:grid-cols-2">
        <DetailTile label="Provider id" value={account.provider_account_id} />
        <DetailTile label="Gmail history cursor" value={account.last_profile_history_id || 'Not captured yet'} />
        <DetailTile label="Access token expires" value={formatDateTime(account.cached_access_token_expires_at)} />
        <div className="rounded-lg border border-hairline bg-page p-3">
          <dt className="text-xs font-semibold uppercase tracking-wide text-ink-secondary">Granted scopes</dt>
          <dd className="mt-1 text-sm text-ink-primary">
            {account.granted_scopes.length === 0 ? 'None recorded' : <ul className="list-inside list-disc space-y-1">{account.granted_scopes.map((scope) => <li key={scope}>{scopeLabel(scope)}</li>)}</ul>}
          </dd>
        </div>
      </dl>

      {disconnectError ? <p role="alert" className="mt-4 rounded-lg border border-accent-red/30 bg-accent-red/10 px-3 py-2 text-sm text-accent-red">{actionErrorMessage(disconnectError, 'Disconnect Gmail')}</p> : null}
    </section>
  );
}

export function ProviderAccountsPage({ client, initialAccount = null, location = window.location, confirmDisconnect = window.confirm }: ProviderAccountsPageProps) {
  const [account, setAccount] = useState<ProviderAccount | null>(initialAccount);
  const callbackNotice = useMemo(() => providerCallbackNotice(location.search), [location.search]);
  const connectGmail = useConnectGmailMutation(client, { onSuccess: (data) => location.assign(data.authorization_url) });
  const disconnectProviderAccount = useDisconnectProviderAccountMutation(client, { onSuccess: (updated) => setAccount(updated) });
  const syncStatuses = useProviderSyncStatuses(client);
  const triggerSync = useTriggerProviderSyncMutation(client);
  const { refetch: refetchSyncStatuses } = syncStatuses;

  useEffect(() => {
    if (callbackNotice) {
      void refetchSyncStatuses();
    }
  }, [callbackNotice, refetchSyncStatuses]);

  const connectedAccount = useMemo(() => account && account.provider_kind === 'gmail' ? account : null, [account]);
  const gmailStatuses = syncStatuses.data?.accounts ?? [];

  function onConnect() {
    connectGmail.mutate();
  }

  function onDisconnect() {
    if (!connectedAccount) return;
    const confirmed = confirmDisconnect(`Disconnect ${connectedAccount.provider_email} from Gmail import? Hail will keep already-imported local mail, but new Gmail mail will stop importing.`);
    if (confirmed) disconnectProviderAccount.mutate(connectedAccount.id);
  }

  const list = (
    <div className="space-y-5">
      <section className="rounded-2xl border border-hairline bg-surface p-5 shadow-sm">
        <div className="flex flex-col gap-4 sm:flex-row sm:items-start sm:justify-between">
          <div>
            <p className="text-xs font-semibold uppercase tracking-wide text-ink-secondary">Provider import mode</p>
            <h2 className="mt-1 text-2xl font-semibold text-ink-primary">Connect Gmail</h2>
            <p className="mt-3 max-w-2xl text-sm leading-6 text-ink-secondary">Gmail remains your public mailbox and spam filter. Hail imports a local Stalwart copy and the normal hail UI reads that local copy. During v1.2 import, hail actions do not archive, delete, mark read, or relabel Gmail mail.</p>
          </div>
          <button type="button" onClick={onConnect} disabled={connectGmail.isPending} className="rounded-full bg-accent-blue px-5 py-2.5 text-sm font-semibold text-white transition hover:bg-accent-blue-hover disabled:cursor-not-allowed disabled:opacity-60">
            {connectGmail.isPending ? 'Opening Google…' : connectedAccount || gmailStatuses.length > 0 ? 'Reconnect Gmail' : 'Connect Gmail'}
          </button>
        </div>

        <div className="mt-5 grid gap-3 sm:grid-cols-3">
          <div className="rounded-lg border border-hairline bg-page p-3"><p className="text-sm font-semibold text-ink-primary">Read-only scope</p><p className="mt-1 text-sm leading-6 text-ink-secondary">Hail requests Gmail read-only access for one-way import.</p></div>
          <div className="rounded-lg border border-hairline bg-page p-3"><p className="text-sm font-semibold text-ink-primary">Tokens stay server-side</p><p className="mt-1 text-sm leading-6 text-ink-secondary">The browser only receives an authorization URL, never OAuth tokens.</p></div>
          <div className="rounded-lg border border-hairline bg-page p-3"><p className="text-sm font-semibold text-ink-primary">Local truth</p><p className="mt-1 text-sm leading-6 text-ink-secondary">Imported messages are viewed and routed from Stalwart, not Gmail labels.</p></div>
        </div>

        {connectGmail.error ? <p role="alert" className="mt-4 rounded-lg border border-accent-red/30 bg-accent-red/10 px-3 py-2 text-sm text-accent-red">{actionErrorMessage(connectGmail.error, 'Connect Gmail')}</p> : null}
      </section>

      {callbackNotice ? (
        callbackNotice.kind === 'connected' ? (
          <p role="status" className="rounded-2xl border border-accent-green/30 bg-accent-green/10 p-4 text-sm text-ink-primary">{callbackNotice.message}</p>
        ) : (
          <p role="alert" className="rounded-2xl border border-accent-red/30 bg-accent-red/10 p-4 text-sm text-accent-red">{callbackNotice.message}</p>
        )
      ) : null}

      {syncStatuses.isPending ? (
        <StateCard title="Checking Gmail import status" body="Loading Gmail sync health from hail-api." className="rounded-2xl border border-hairline bg-surface p-8 text-center" />
      ) : syncStatuses.isError ? (
        <p role="alert" className="rounded-2xl border border-accent-red/30 bg-accent-red/10 p-4 text-sm text-accent-red">{actionErrorMessage(syncStatuses.error, 'Load Gmail import status')}</p>
      ) : gmailStatuses.length > 0 ? (
        gmailStatuses.map((status) => <ProviderSyncStatusCard key={status.id} status={status} syncing={triggerSync.isPending && triggerSync.variables === status.id} syncError={triggerSync.variables === status.id ? triggerSync.error : null} onSync={() => triggerSync.mutate(status.id)} />)
      ) : connectedAccount ? (
        <ProviderAccountCard account={connectedAccount} disconnecting={disconnectProviderAccount.isPending} disconnectError={disconnectProviderAccount.error} onDisconnect={onDisconnect} />
      ) : (
        <StateCard title="No Gmail account connected" body="Connect Gmail to start the server-side OAuth flow. Once Google redirects back, import status will appear here." className="rounded-2xl border border-hairline bg-surface p-8 text-center" />
      )}

      {connectedAccount && gmailStatuses.length > 0 ? <ProviderAccountCard account={connectedAccount} disconnecting={disconnectProviderAccount.isPending} disconnectError={disconnectProviderAccount.error} onDisconnect={onDisconnect} /> : null}
    </div>
  );

  return <AppShell title="Provider Accounts" description="Connect Gmail for one-way provider import into hail." list={list} wide />;
}
