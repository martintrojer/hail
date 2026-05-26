import { useMemo, useState } from 'react';
import type { HailApiClient, ProviderAccount } from '../api/client';
import {
  useConnectGmailMutation,
  useDisconnectProviderAccountMutation,
} from '../api/query';
import { StateCard } from '../components/StateCard';
import { AppShell } from '../layout/AppShell';
import { actionErrorMessage } from '../lib/errorMessages';

interface ProviderAccountsPageProps {
  client?: HailApiClient;
  initialAccount?: ProviderAccount | null;
  location?: Pick<Location, 'assign'>;
  confirmDisconnect?: (message: string) => boolean;
}

const GMAIL_READONLY_SCOPE = 'https://www.googleapis.com/auth/gmail.readonly';

function statusLabel(status: string) {
  switch (status) {
    case 'active':
      return 'Connected';
    case 'paused':
      return 'Paused';
    case 'disconnected':
      return 'Disconnected';
    case 'revoked':
      return 'Revoked';
    default:
      return status || 'Unknown';
  }
}

function formatDateTime(value: string | null | undefined) {
  if (!value) {
    return 'Not available yet';
  }

  return new Intl.DateTimeFormat(undefined, {
    dateStyle: 'medium',
    timeStyle: 'short',
  }).format(new Date(value));
}

function scopeLabel(scope: string) {
  if (scope === GMAIL_READONLY_SCOPE) {
    return 'Gmail read-only import';
  }
  return scope;
}

function ProviderAccountCard({
  account,
  disconnecting,
  disconnectError,
  onDisconnect,
}: {
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
          <h2 className="mt-1 text-xl font-semibold text-ink-primary">
            {account.display_email || account.provider_email}
          </h2>
          <div className="mt-3 inline-flex items-center gap-2 rounded-full border border-hairline bg-page px-3 py-1 text-sm font-medium text-ink-primary">
            <span
              aria-hidden="true"
              className={`h-2.5 w-2.5 rounded-full ${connected ? 'bg-accent-green' : 'bg-ink-tertiary'}`}
            />
            {statusLabel(account.sync_status)}
          </div>
        </div>
        <button
          type="button"
          onClick={onDisconnect}
          disabled={disconnecting || !connected}
          className="rounded-full border border-accent-red/40 px-4 py-2 text-sm font-semibold text-accent-red transition hover:bg-accent-red/10 disabled:cursor-not-allowed disabled:opacity-60"
        >
          {disconnecting ? 'Disconnecting…' : connected ? 'Disconnect' : 'Disconnected'}
        </button>
      </div>

      <dl className="mt-5 grid gap-3 sm:grid-cols-2">
        <div className="rounded-lg border border-hairline bg-page p-3">
          <dt className="text-xs font-semibold uppercase tracking-wide text-ink-secondary">Provider id</dt>
          <dd className="mt-1 break-all text-sm text-ink-primary">{account.provider_account_id}</dd>
        </div>
        <div className="rounded-lg border border-hairline bg-page p-3">
          <dt className="text-xs font-semibold uppercase tracking-wide text-ink-secondary">Gmail history cursor</dt>
          <dd className="mt-1 break-all text-sm text-ink-primary">
            {account.last_profile_history_id || 'Not captured yet'}
          </dd>
        </div>
        <div className="rounded-lg border border-hairline bg-page p-3">
          <dt className="text-xs font-semibold uppercase tracking-wide text-ink-secondary">Access token expires</dt>
          <dd className="mt-1 text-sm text-ink-primary">
            {formatDateTime(account.cached_access_token_expires_at)}
          </dd>
        </div>
        <div className="rounded-lg border border-hairline bg-page p-3">
          <dt className="text-xs font-semibold uppercase tracking-wide text-ink-secondary">Granted scopes</dt>
          <dd className="mt-1 text-sm text-ink-primary">
            {account.granted_scopes.length === 0 ? (
              'None recorded'
            ) : (
              <ul className="list-inside list-disc space-y-1">
                {account.granted_scopes.map((scope) => (
                  <li key={scope}>{scopeLabel(scope)}</li>
                ))}
              </ul>
            )}
          </dd>
        </div>
      </dl>

      {disconnectError ? (
        <p role="alert" className="mt-4 rounded-lg border border-accent-red/30 bg-accent-red/10 px-3 py-2 text-sm text-accent-red">
          {actionErrorMessage(disconnectError, 'Disconnect Gmail')}
        </p>
      ) : null}
    </section>
  );
}

export function ProviderAccountsPage({
  client,
  initialAccount = null,
  location = window.location,
  confirmDisconnect = window.confirm,
}: ProviderAccountsPageProps) {
  const [account, setAccount] = useState<ProviderAccount | null>(initialAccount);
  const connectGmail = useConnectGmailMutation(client, {
    onSuccess: (data) => {
      location.assign(data.authorization_url);
    },
  });
  const disconnectProviderAccount = useDisconnectProviderAccountMutation(client, {
    onSuccess: (updated) => setAccount(updated),
  });

  const connectedAccount = useMemo(
    () => account && account.provider_kind === 'gmail' ? account : null,
    [account],
  );

  function onConnect() {
    connectGmail.mutate();
  }

  function onDisconnect() {
    if (!connectedAccount) {
      return;
    }

    const confirmed = confirmDisconnect(
      `Disconnect ${connectedAccount.provider_email} from Gmail import? Hail will keep already-imported local mail, but new Gmail mail will stop importing.`,
    );
    if (!confirmed) {
      return;
    }

    disconnectProviderAccount.mutate(connectedAccount.id);
  }

  const list = (
    <div className="space-y-5">
      <section className="rounded-2xl border border-hairline bg-surface p-5 shadow-sm">
        <div className="flex flex-col gap-4 sm:flex-row sm:items-start sm:justify-between">
          <div>
            <p className="text-xs font-semibold uppercase tracking-wide text-ink-secondary">Provider import mode</p>
            <h2 className="mt-1 text-2xl font-semibold text-ink-primary">Connect Gmail</h2>
            <p className="mt-3 max-w-2xl text-sm leading-6 text-ink-secondary">
              Gmail remains your public mailbox and spam filter. Hail imports a local Stalwart copy and the normal hail UI reads that local copy. During v1.2 import, hail actions do not archive, delete, mark read, or relabel Gmail mail.
            </p>
          </div>
          <button
            type="button"
            onClick={onConnect}
            disabled={connectGmail.isPending}
            className="rounded-full bg-accent-blue px-5 py-2.5 text-sm font-semibold text-white transition hover:bg-accent-blue-hover disabled:cursor-not-allowed disabled:opacity-60"
          >
            {connectGmail.isPending ? 'Opening Google…' : connectedAccount ? 'Reconnect Gmail' : 'Connect Gmail'}
          </button>
        </div>

        <div className="mt-5 grid gap-3 sm:grid-cols-3">
          <div className="rounded-lg border border-hairline bg-page p-3">
            <p className="text-sm font-semibold text-ink-primary">Read-only scope</p>
            <p className="mt-1 text-sm leading-6 text-ink-secondary">Hail requests Gmail read-only access for one-way import.</p>
          </div>
          <div className="rounded-lg border border-hairline bg-page p-3">
            <p className="text-sm font-semibold text-ink-primary">Tokens stay server-side</p>
            <p className="mt-1 text-sm leading-6 text-ink-secondary">The browser only receives an authorization URL, never OAuth tokens.</p>
          </div>
          <div className="rounded-lg border border-hairline bg-page p-3">
            <p className="text-sm font-semibold text-ink-primary">Local truth</p>
            <p className="mt-1 text-sm leading-6 text-ink-secondary">Imported messages are viewed and routed from Stalwart, not Gmail labels.</p>
          </div>
        </div>

        {connectGmail.error ? (
          <p role="alert" className="mt-4 rounded-lg border border-accent-red/30 bg-accent-red/10 px-3 py-2 text-sm text-accent-red">
            {actionErrorMessage(connectGmail.error, 'Connect Gmail')}
          </p>
        ) : null}
      </section>

      {connectedAccount ? (
        <ProviderAccountCard
          account={connectedAccount}
          disconnecting={disconnectProviderAccount.isPending}
          disconnectError={disconnectProviderAccount.error}
          onDisconnect={onDisconnect}
        />
      ) : (
        <StateCard
          title="No Gmail account connected"
          body="Connect Gmail to start the server-side OAuth flow. Once Google redirects back, import status will appear here when the status API is available."
          className="rounded-2xl border border-hairline bg-surface p-8 text-center"
        />
      )}
    </div>
  );

  return (
    <AppShell
      title="Provider Accounts"
      description="Connect Gmail for one-way provider import into hail."
      list={list}
      wide
    />
  );
}
