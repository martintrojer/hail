import type { HailApiClient, SpeakeasyState } from '../api/client';
import {
  useRotateSpeakeasyMutation,
  useSpeakeasy,
} from '../api/query';
import { ErrorState } from '../components/ErrorState';
import { LoadingState } from '../components/LoadingState';
import { AppShell } from '../layout/AppShell';
import { pillButtonClass } from '../lib/buttonStyles';
import { formatDate, formatFullDateTime } from '../lib/dates';
import { actionErrorMessage, viewErrorMessage } from '../lib/errorMessages';

interface ScreenerSpeakeasyPageProps {
  client?: HailApiClient;
}

function formatPeriod(period: string) {
  const [year, month] = period.split('-').map(Number);
  if (!year || !month) {
    return period;
  }

  return new Intl.DateTimeFormat(undefined, {
    month: 'long',
    year: 'numeric',
    timeZone: 'UTC',
  }).format(new Date(Date.UTC(year, month - 1, 1)));
}

function SpeakeasyIntro() {
  return (
    <section className="rounded-2xl border border-border-subtle bg-bg-surface p-5">
      <p className="text-xs font-semibold uppercase tracking-[0.18em] text-ink-tertiary">
        Screener Speakeasy
      </p>
      <h2 className="mt-2 text-xl font-semibold text-ink-primary">
        A monthly passphrase for one-message bypasses.
      </h2>
      <p className="mt-2 max-w-2xl text-sm leading-6 text-ink-secondary">
        Share this passphrase when someone needs to get a single message past
        The Screener. A matching message skips the Screener once; it does not
        approve the sender, create a rule, or choose where future mail goes.
      </p>
    </section>
  );
}

function PassphraseCard({ speakeasy }: { speakeasy: SpeakeasyState }) {
  return (
    <section className="rounded-2xl bg-bg-surface p-5 shadow-sm">
      <div className="flex flex-col gap-4 sm:flex-row sm:items-start sm:justify-between">
        <div>
          <p className="text-xs font-semibold uppercase tracking-[0.18em] text-ink-tertiary">
            Current passphrase
          </p>
          <h3 className="mt-2 text-lg font-semibold text-ink-primary">
            {formatPeriod(speakeasy.period)}
          </h3>
        </div>
        <span className="rounded-full border border-accent-blue/30 bg-accent-blue/10 px-3 py-1 text-xs font-semibold text-accent-blue">
          One message only
        </span>
      </div>

      <input
        readOnly
        aria-label="Current Speakeasy passphrase"
        value={speakeasy.passphrase}
        onFocus={(event) => event.currentTarget.select()}
        className="mt-5 w-full rounded-xl border border-border-menu bg-bg-page px-4 py-3 font-mono text-base font-semibold tracking-wide text-ink-primary outline-none ring-accent-blue transition focus:border-accent-blue focus:ring-2"
      />

      <dl className="mt-5 grid gap-3 text-sm sm:grid-cols-3">
        <div className="rounded-lg border border-border-subtle bg-bg-page p-3">
          <dt className="text-xs font-semibold uppercase tracking-wide text-ink-tertiary">
            Rotates
          </dt>
          <dd className="mt-1 font-medium text-ink-primary">
            {formatDate(speakeasy.rotates_at)}
          </dd>
        </div>
        <div className="rounded-lg border border-border-subtle bg-bg-page p-3">
          <dt className="text-xs font-semibold uppercase tracking-wide text-ink-tertiary">
            Generated
          </dt>
          <dd className="mt-1 font-medium text-ink-primary">
            {formatFullDateTime(speakeasy.generated_at)}
          </dd>
        </div>
        <div className="rounded-lg border border-border-subtle bg-bg-page p-3">
          <dt className="text-xs font-semibold uppercase tracking-wide text-ink-tertiary">
            Manual rotation
          </dt>
          <dd className="mt-1 font-medium text-ink-primary">
            {speakeasy.manually_rotated_at
              ? formatFullDateTime(speakeasy.manually_rotated_at)
              : 'Not rotated this period'}
          </dd>
        </div>
      </dl>
    </section>
  );
}

function HowItWorks() {
  return (
    <section className="rounded-2xl border border-border-subtle bg-bg-surface p-5">
      <h3 className="text-base font-semibold text-ink-primary">How to use it</h3>
      <ol className="mt-3 list-decimal space-y-2 pl-5 text-sm leading-6 text-ink-secondary">
        <li>Put the current passphrase in the subject or body of a message.</li>
        <li>Only that matching message skips the Screener.</li>
        <li>Future messages from the same sender still go through normal screening.</li>
      </ol>
      <p className="mt-4 rounded-lg border border-amber-300/40 bg-amber-100/40 px-3 py-2 text-sm leading-6 text-ink-secondary">
        Treat this like a shared secret. Regenerating it immediately invalidates
        the previous passphrase for new incoming messages.
      </p>
    </section>
  );
}

function RotateSection({ client }: { client?: HailApiClient }) {
  const rotate = useRotateSpeakeasyMutation(client);

  return (
    <section className="rounded-2xl border border-border-subtle bg-bg-surface p-5">
      <div className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
        <div>
          <h3 className="text-base font-semibold text-ink-primary">
            Need a new passphrase now?
          </h3>
          <p className="mt-1 max-w-2xl text-sm leading-6 text-ink-secondary">
            Rotate early if the current phrase was shared too broadly. The old
            phrase stops working immediately.
          </p>
        </div>
        <button
          type="button"
          onClick={() => rotate.mutate()}
          disabled={rotate.isPending}
          className={`${pillButtonClass('primary', 'md')} self-start sm:self-auto`}
        >
          {rotate.isPending ? 'Regenerating…' : 'Regenerate passphrase'}
        </button>
      </div>
      {rotate.isError ? (
        <p role="alert" className="mt-3 text-sm text-accent-red">
          {actionErrorMessage(rotate.error, 'Speakeasy rotation')}
        </p>
      ) : null}
    </section>
  );
}

export function ScreenerSpeakeasyPage({ client }: ScreenerSpeakeasyPageProps) {
  const query = useSpeakeasy(client);

  let body;
  if (query.isPending) {
    body = <LoadingState label="Loading Speakeasy passphrase" />;
  } else if (query.isError) {
    body = (
      <ErrorState
        message={viewErrorMessage(query.error, 'Speakeasy')}
        onRetry={() => void query.refetch()}
      />
    );
  } else {
    body = (
      <div className="space-y-5">
        <SpeakeasyIntro />
        <PassphraseCard speakeasy={query.data.speakeasy} />
        <HowItWorks />
        <RotateSection client={client} />
      </div>
    );
  }

  return (
    <AppShell
      title="Speakeasy"
      description="Monthly passphrase for one-message Screener bypasses."
      list={body}
    />
  );
}
