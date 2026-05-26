import { RouterProvider } from '@tanstack/react-router';
import { cleanup, screen } from '@testing-library/react';
import type { ReactNode } from 'react';
import { afterEach, describe, expect, it } from 'vitest';
import type { AttachmentsResponse } from '../api/client';
import { HailApiError } from '../api/client';
import { AuthProvider } from '../auth/AuthProvider';
import { router } from '../router';
import { createTestQueryClient, renderWithQueryClient, seedMe, TestHailApiClient } from '../test-utils';
import { AllFilesPage } from './AllFilesPage';

class AllFilesTestClient extends TestHailApiClient {
  failure: Error | null = null;
  private response: AttachmentsResponse;

  constructor(response: AttachmentsResponse = sampleAttachments()) {
    super();
    this.response = response;
  }

  override async listAttachments(): Promise<AttachmentsResponse> {
    if (this.failure) {
      throw this.failure;
    }
    return this.response;
  }
}

function sampleAttachments(): AttachmentsResponse {
  return {
    items: [
      {
        blob_id: 'blob-invoice',
        name: 'invoice.pdf',
        type: 'application/pdf',
        size: 1536,
        download_url: '/api/attachments/blob-invoice/download',
        context: {
          thread_id: 'thread-billing',
          email_id: 'email-billing',
          subject: 'May invoice',
          from: 'Billing <billing@example.com>',
          received_at: '2026-05-24T12:00:00Z',
          preview: 'Your invoice is attached.',
        },
      },
      {
        blob_id: 'blob-photo',
        name: 'photo.jpg',
        type: 'image/jpeg',
        size: 2_097_152,
        download_url: '/api/attachments/blob-photo/download',
        context: {
          thread_id: 'thread-family',
          email_id: 'email-family',
          subject: 'Weekend photos',
          from: 'Ari <ari@example.com>',
          received_at: null,
          preview: '',
        },
      },
    ],
  };
}

function response(status: number) {
  return new Response(JSON.stringify({ error: 'boom' }), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

let currentTestBody: ReactNode = null;
let restoreFilesRoute: (() => void) | null = null;

function restoreRoute() {
  restoreFilesRoute?.();
  restoreFilesRoute = null;
}

function TestBody() {
  return currentTestBody;
}

function installTestRouteComponent() {
  const matchRoute = router.routesByPath['/files'];
  const previousComponent = matchRoute.options.component;
  const previousBeforeLoad = matchRoute.options.beforeLoad;
  matchRoute.options.component = TestBody;
  matchRoute.options.beforeLoad = undefined;
  restoreFilesRoute = () => {
    matchRoute.options.component = previousComponent;
    matchRoute.options.beforeLoad = previousBeforeLoad;
  };
}

function renderPage(client = new AllFilesTestClient()) {
  const queryClient = createTestQueryClient();
  seedMe(queryClient, client.testUser);
  currentTestBody = (
    <AuthProvider>
      <AllFilesPage client={client} />
    </AuthProvider>
  );
  installTestRouteComponent();
  window.history.pushState({}, '', '/files');
  renderWithQueryClient(<RouterProvider router={router} />, queryClient);
  return client;
}

afterEach(() => {
  currentTestBody = null;
  restoreRoute();
  window.history.pushState({}, '', '/');
  cleanup();
});

describe('AllFilesPage', () => {
  it('lists attachments with context and download links', async () => {
    renderPage();

    expect(await screen.findByRole('heading', { name: 'All Files' })).toBeInTheDocument();
    expect(await screen.findByText('invoice.pdf')).toBeInTheDocument();
    expect(screen.getByText('May invoice')).toBeInTheDocument();
    expect(screen.getByText('Billing <billing@example.com>')).toBeInTheDocument();
    expect(screen.getAllByText('2')).toHaveLength(2);
    expect(screen.getByText('2.0 MB')).toBeInTheDocument();

    const openLinks = screen.getAllByRole('link', { name: 'Open' });
    expect(openLinks[0]).toHaveAttribute('href', '/api/attachments/blob-invoice/download');
    expect(screen.getAllByRole('link', { name: 'Download' })[1]).toHaveAttribute(
      'download',
      'photo.jpg',
    );
    expect(screen.getByRole('link', { name: 'Weekend photos' })).toHaveAttribute(
      'href',
      '/thread/thread-family',
    );
  });

  it('shows empty and error states', async () => {
    renderPage(new AllFilesTestClient({ items: [] }));
    expect(await screen.findByText('No files yet')).toBeInTheDocument();
    cleanup();

    const client = new AllFilesTestClient();
    client.failure = new HailApiError(500, undefined, response(500));
    renderPage(client);

    expect(await screen.findByText('Something went wrong.')).toBeInTheDocument();
    expect(screen.getByText('All Files failed with HTTP 500.')).toBeInTheDocument();
  });
});
