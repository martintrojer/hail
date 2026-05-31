import { cleanup, fireEvent, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it } from 'vitest';
import type { UpdateUserPrefsRequest } from '../api/client';
import { createTestQueryClient, renderWithQueryClient, TestHailApiClient } from '../test-utils';
import { PreferencesPanel } from './PreferencesPage';

class PreferencesTestClient extends TestHailApiClient {
  prefs = { feed_load_remote_images: false };
  updates: UpdateUserPrefsRequest[] = [];

  override async getUserPrefs() {
    return this.prefs;
  }

  override async updateUserPrefs(body: UpdateUserPrefsRequest) {
    this.updates.push(body);
    this.prefs = {
      feed_load_remote_images: Boolean(body.feed_load_remote_images),
    };
    return this.prefs;
  }
}

afterEach(() => cleanup());

function renderPreferences(client: PreferencesTestClient) {
  renderWithQueryClient(
    <PreferencesPanel client={client} />,
    createTestQueryClient(),
  );
}

describe('PreferencesPage', () => {
  it('saves the newsletter remote images preference', async () => {
    const client = new PreferencesTestClient();
    renderPreferences(client);

    const toggle = await screen.findByRole('switch', {
      name: 'Load remote images in newsletters',
    });
    expect(toggle).not.toBeChecked();

    fireEvent.click(toggle);

    await waitFor(() => {
      expect(client.updates).toEqual([{ feed_load_remote_images: true }]);
    });
    expect(toggle).toBeChecked();
    expect(screen.getByText(/Tracker pixels and known tracking domains are still blocked/i)).toBeInTheDocument();
  });
});
