import { useEffect, useState } from 'react';
import type { HailApiClient } from '../api/client';
import { useUpdateUserPrefsMutation, useUserPrefs } from '../api/query';
import { useApiClient } from '../api/ApiClientProvider';
import { AppShell } from '../layout/AppShell';
import { ErrorState } from '../components/ErrorState';
import { LoadingState } from '../components/LoadingState';
import { Alert, AlertDescription } from '../components/ui/alert';
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '../components/ui/card';
import { Switch } from '../components/ui/switch';
import { formErrorMessage, viewErrorMessage } from '../lib/errorMessages';

interface PreferencesPageProps {
  client?: HailApiClient;
}

export function PreferencesPanel({ client }: { client: HailApiClient }) {
  const prefs = useUserPrefs(client);
  const updatePrefs = useUpdateUserPrefsMutation(client);
  const [checked, setChecked] = useState(false);

  useEffect(() => {
    if (prefs.data) {
      setChecked(prefs.data.feed_load_remote_images);
    }
  }, [prefs.data]);

  if (prefs.isPending) {
    return <LoadingState label="Loading preferences" />;
  }

  if (prefs.isError) {
    return (
      <ErrorState
        message={viewErrorMessage(prefs.error, 'Preferences')}
        onRetry={() => void prefs.refetch()}
      />
    );
  }

  function onToggle(next: boolean) {
    setChecked(next);
    updatePrefs.mutate(
      { feed_load_remote_images: next },
      {
        onError: () => setChecked(!next),
      },
    );
  }

  return (
    <Card size="sm" className="max-w-2xl">
      <CardHeader>
        <CardTitle role="heading" aria-level={2}>Newsletter privacy</CardTitle>
        <CardDescription>
          Control whether Feed newsletter cards load remote images by default.
        </CardDescription>
      </CardHeader>
      <CardContent>
        <div className="flex items-start justify-between gap-4">
          <div className="space-y-1">
            <label htmlFor="feed-load-remote-images" className="text-sm font-medium text-foreground">
              Load remote images in newsletters
            </label>
            <p className="max-w-xl text-sm text-muted-foreground">
              Off by default for privacy. Remote images can reveal your IP address and that you opened
              a message. Tracker pixels and known tracking domains are still blocked when this is on.
            </p>
          </div>
          <Switch
            id="feed-load-remote-images"
            checked={checked}
            disabled={updatePrefs.isPending}
            onCheckedChange={onToggle}
            aria-label="Load remote images in newsletters"
          />
        </div>
        {updatePrefs.isError ? (
          <Alert variant="destructive" className="mt-4">
            <AlertDescription>
              {formErrorMessage(updatePrefs.error, 'Could not save preference.')}
            </AlertDescription>
          </Alert>
        ) : null}
      </CardContent>
    </Card>
  );
}

export function PreferencesPage({ client }: PreferencesPageProps) {
  const contextClient = useApiClient();
  const apiClient = client ?? contextClient;

  return (
    <AppShell
      title="Preferences"
      list={<PreferencesPanel client={apiClient} />}
    />
  );
}
