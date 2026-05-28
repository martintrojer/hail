import type { HailApiClient, SpeakeasyState } from '../api/client';
import {
  useRotateSpeakeasyMutation,
  useSpeakeasy,
} from '../api/query';
import { Alert, AlertDescription, AlertTitle } from '../components/ui/alert';
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
import { Field, FieldDescription, FieldGroup, FieldLabel } from '../components/ui/field';
import { Input } from '../components/ui/input';
import { ErrorState } from '../components/ErrorState';
import { LoadingState } from '../components/LoadingState';
import { AppShell } from '../layout/AppShell';
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
    <Card>
      <CardHeader>
        <CardDescription className="text-xs font-semibold uppercase tracking-[0.18em]">
          Screener Speakeasy
        </CardDescription>
        <CardTitle className="text-xl" role="heading" aria-level={2}>
          A monthly passphrase for one-message bypasses.
        </CardTitle>
        <CardDescription className="max-w-2xl leading-6">
          Share this passphrase when someone needs to get a single message past
          The Screener. A matching message skips the Screener once; it does not
          approve the sender, create a rule, or choose where future mail goes.
        </CardDescription>
      </CardHeader>
    </Card>
  );
}

function PassphraseCard({ speakeasy }: { speakeasy: SpeakeasyState }) {
  return (
    <Card>
      <CardHeader>
        <CardDescription className="text-xs font-semibold uppercase tracking-[0.18em]">
          Current passphrase
        </CardDescription>
        <CardTitle>{formatPeriod(speakeasy.period)}</CardTitle>
        <CardAction>
          <Badge variant="secondary">One message only</Badge>
        </CardAction>
      </CardHeader>

      <CardContent>
        <FieldGroup>
          <Field>
            <FieldLabel htmlFor="speakeasy-passphrase" className="sr-only">
              Current Speakeasy passphrase
            </FieldLabel>
            <Input
              id="speakeasy-passphrase"
              readOnly
              aria-label="Current Speakeasy passphrase"
              value={speakeasy.passphrase}
              onFocus={(event) => event.currentTarget.select()}
              className="h-12 bg-muted font-mono text-base font-semibold tracking-wide"
            />
            <FieldDescription>
              Select and copy this phrase when you need to share a one-message
              bypass.
            </FieldDescription>
          </Field>
        </FieldGroup>

        <dl className="mt-5 grid gap-3 text-sm sm:grid-cols-3">
          <Card size="sm" className="bg-muted/40 shadow-none">
            <CardHeader>
              <CardDescription className="text-xs font-semibold uppercase tracking-wide">
                Rotates
              </CardDescription>
              <CardTitle className="text-sm">
                {formatDate(speakeasy.rotates_at)}
              </CardTitle>
            </CardHeader>
          </Card>
          <Card size="sm" className="bg-muted/40 shadow-none">
            <CardHeader>
              <CardDescription className="text-xs font-semibold uppercase tracking-wide">
                Generated
              </CardDescription>
              <CardTitle className="text-sm">
                {formatFullDateTime(speakeasy.generated_at)}
              </CardTitle>
            </CardHeader>
          </Card>
          <Card size="sm" className="bg-muted/40 shadow-none">
            <CardHeader>
              <CardDescription className="text-xs font-semibold uppercase tracking-wide">
                Manual rotation
              </CardDescription>
              <CardTitle className="text-sm">
                {speakeasy.manually_rotated_at
                  ? formatFullDateTime(speakeasy.manually_rotated_at)
                  : 'Not rotated this period'}
              </CardTitle>
            </CardHeader>
          </Card>
        </dl>
      </CardContent>
    </Card>
  );
}

function HowItWorks() {
  return (
    <Card>
      <CardHeader>
        <CardTitle>How to use it</CardTitle>
      </CardHeader>
      <CardContent className="flex flex-col gap-4">
        <ol className="list-decimal pl-5 text-sm leading-6 text-muted-foreground">
          <li>Put the current passphrase in the subject or body of a message.</li>
          <li>Only that matching message skips the Screener.</li>
          <li>Future messages from the same sender still go through normal screening.</li>
        </ol>
        <Alert>
          <AlertTitle>Treat this like a shared secret.</AlertTitle>
          <AlertDescription>
            Regenerating it immediately invalidates the previous passphrase for
            new incoming messages.
          </AlertDescription>
        </Alert>
      </CardContent>
    </Card>
  );
}

function RotateSection({ client }: { client?: HailApiClient }) {
  const rotate = useRotateSpeakeasyMutation(client);

  return (
    <Card>
      <CardHeader>
        <CardTitle>Need a new passphrase now?</CardTitle>
        <CardDescription className="max-w-2xl leading-6">
          Rotate early if the current phrase was shared too broadly. The old
          phrase stops working immediately.
        </CardDescription>
        <CardAction>
          <Button
            type="button"
            onClick={() => rotate.mutate()}
            disabled={rotate.isPending}
          >
            {rotate.isPending ? 'Regenerating…' : 'Regenerate passphrase'}
          </Button>
        </CardAction>
      </CardHeader>
      {rotate.isError ? (
        <CardContent>
          <Alert variant="destructive">
            <AlertDescription>
              {actionErrorMessage(rotate.error, 'Speakeasy rotation')}
            </AlertDescription>
          </Alert>
        </CardContent>
      ) : null}
    </Card>
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
      <div className="flex flex-col gap-5">
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
