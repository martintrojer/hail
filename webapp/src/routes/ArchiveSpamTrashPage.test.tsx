import { RouterProvider } from '@tanstack/react-router';
import { cleanup, fireEvent, screen, waitFor, within } from '@testing-library/react';
import type { ReactNode } from 'react';
import { afterEach, describe, expect, it } from 'vitest';
import type { MailViewItem, MailViewResponse, ThreadVerbResponse } from '../api/client';
import { ApiClientProvider } from '../api/ApiClientProvider';
import { AuthProvider } from '../auth/AuthProvider';
import { UndoToastProvider } from '../components/UndoToastProvider';
import { router } from '../router';
import {
  createTestQueryClient,
  installTestRoute,
  renderWithQueryClient,
  seedMe,
  TestHailApiClient,
} from '../test-utils';
import { ArchivePage } from './ArchivePage';
import { SpamPage } from './SpamPage';
import { TrashPage } from './TrashPage';

type ArchiveRouteKind = 'archive' | 'spam' | 'trash';

type RouteCase = {
  kind: ArchiveRouteKind;
  path: '/archive' | '/spam' | '/trash';
  title: string;
  render: (client: ArchiveSpamTrashTestClient) => ReactNode;
};

class ArchiveSpamTrashTestClient extends TestHailApiClient {
  readonly calls: ArchiveRouteKind[] = [];
  readonly classifyCalls: Array<{ threadId: string; to: 'imbox' }> = [];
  readonly notSpamCalls: string[] = [];
  readonly restoreCalls: string[] = [];
  readonly destroyCalls: string[] = [];

  constructor(private readonly items: MailViewItem[]) {
    super();
  }

  override async getArchiveView(): Promise<MailViewResponse> {
    this.calls.push('archive');
    return { items: this.items, next_cursor: null };
  }

  override async getSpamView(): Promise<MailViewResponse> {
    this.calls.push('spam');
    return { items: this.items, next_cursor: null };
  }

  override async getTrash(): Promise<MailViewResponse> {
    this.calls.push('trash');
    return { items: this.items, next_cursor: null };
  }

  override async classifyThread(threadId: string, to: 'imbox'): Promise<ThreadVerbResponse> {
    this.classifyCalls.push({ threadId, to });
    return this.threadVerbResponse('classify');
  }

  override async notSpamThread(threadId: string): Promise<ThreadVerbResponse> {
    this.notSpamCalls.push(threadId);
    return this.threadVerbResponse('not-spam');
  }

  override async restoreThread(threadId: string): Promise<ThreadVerbResponse> {
    this.restoreCalls.push(threadId);
    return this.threadVerbResponse('restore');
  }

  override async destroyThread(threadId: string): Promise<{ status: string }> {
    this.destroyCalls.push(threadId);
    return { status: 'deleted' };
  }

  private threadVerbResponse(action: string): ThreadVerbResponse {
    return {
      undo: {
        id: `undo-${action}`,
        action: 'thread.stack',
        expires_at: '2026-05-23T13:00:00Z',
      },
    };
  }
}

const routeCases: RouteCase[] = [
  {
    kind: 'archive',
    path: '/archive',
    title: 'Archive',
    render: (client) => <ArchivePage client={client} />,
  },
  {
    kind: 'spam',
    path: '/spam',
    title: 'Spam',
    render: (client) => <SpamPage client={client} />,
  },
  {
    kind: 'trash',
    path: '/trash',
    title: 'Trash',
    render: (client) => <TrashPage client={client} />,
  },
];

let currentTestBody: ReactNode = null;
let restoreRoute: (() => void) | null = null;

function TestBody() {
  return currentTestBody;
}

afterEach(() => {
  currentTestBody = null;
  restoreRoute?.();
  restoreRoute = null;
  window.history.pushState({}, '', '/');
  cleanup();
});

function mailItem(overrides: Partial<MailViewItem> = {}): MailViewItem {
  return {
    thread_id: 'thread/archive-row',
    email_id: 'email-archive-row',
    from: 'Alice Sender',
    to: ['reader@example.com'],
    cc: [],
    bcc: [],
    subject: 'Quarterly archive update',
    preview: 'Compact row details from Alice.',
    received_at: '2026-05-23T12:00:00Z',
    unread: true,
    classification: 'imbox',
    has_notes: true,
    labels: [
      {
        id: 12,
        name: 'Work/Receipts',
        leaf_name: 'Receipts',
        path_segments: ['Work', 'Receipts'],
        source: 'gmail',
        color: null,
        thread_count: 8,
      },
    ],
    feed_html: null,
    feed_blocked_trackers: null,
    ...overrides,
  };
}

function renderRoute(route: RouteCase, client = new ArchiveSpamTrashTestClient([mailItem()])) {
  const queryClient = createTestQueryClient();
  seedMe(queryClient);

  currentTestBody = (
    <ApiClientProvider client={client}>
      <AuthProvider>
        <UndoToastProvider>{route.render(client)}</UndoToastProvider>
      </AuthProvider>
    </ApiClientProvider>
  );
  restoreRoute = installTestRoute(router, route.path, {
    component: TestBody,
    beforeLoad: undefined,
  });
  window.history.pushState({}, '', route.path);

  renderWithQueryClient(<RouterProvider router={router} />, queryClient);

  return client;
}

describe.each(routeCases)('$title page', (route) => {
  it('renders the shared compact mail row with labels and ThreadLink metadata', async () => {
    const client = renderRoute(route);

    const link = await screen.findByRole('link', {
      name: 'Open Quarterly archive update from Alice Sender',
    });
    expect(client.calls).toEqual([route.kind]);
    expect(link).toHaveAttribute('href', '/thread/thread%2Farchive-row');
    expect(link).toHaveAttribute('data-hail-mail-list-item', 'true');
    expect(link).toHaveAttribute('data-hail-thread-id', 'thread/archive-row');
    expect(link).toHaveClass('py-1');
    expect(link.className).not.toContain('py-4');
    expect(link.className).not.toContain('sm:py-5');
    expect(within(link).getByText('Compact row details from Alice.')).toBeInTheDocument();
    expect(within(link).getByLabelText('Thread has notes')).toBeInTheDocument();
    expect(within(link).getByText('Receipts')).toHaveAttribute('title', 'Work/Receipts');
    expect(within(link).getByLabelText('Label Work/Receipts')).toBeInTheDocument();
  });
});

describe('Archive/Spam/Trash actions', () => {
  it('restores archived mail by classifying it to Imbox', async () => {
    const client = renderRoute(routeCases[0]);
    const link = await screen.findByRole('link', { name: 'Open Quarterly archive update from Alice Sender' });

    fireEvent.click(within(link).getByRole('checkbox', { name: 'Select Alice Sender' }));
    fireEvent.click(screen.getByRole('button', { name: 'Restore' }));

    await waitFor(() => expect(client.classifyCalls).toEqual([{ threadId: 'thread/archive-row', to: 'imbox' }]));
    expect(client.restoreCalls).toEqual([]);
  });

  it('keeps Spam verbs wired to not-spam and delete forever', async () => {
    const client = renderRoute(routeCases[1]);
    const link = await screen.findByRole('link', { name: 'Open Quarterly archive update from Alice Sender' });

    fireEvent.click(within(link).getByRole('checkbox', { name: 'Select Alice Sender' }));
    fireEvent.click(screen.getByRole('button', { name: 'Not Spam' }));
    await waitFor(() => expect(client.notSpamCalls).toEqual(['thread/archive-row']));

    fireEvent.click(within(link).getByRole('checkbox', { name: 'Deselect Alice Sender' }));
    fireEvent.click(within(link).getByRole('checkbox', { name: 'Select Alice Sender' }));
    fireEvent.click(screen.getByRole('button', { name: 'Delete forever' }));
    await waitFor(() => expect(client.destroyCalls).toEqual(['thread/archive-row']));
  });

  it('restores trash through the restore endpoint and can delete forever', async () => {
    const client = renderRoute(routeCases[2]);
    const link = await screen.findByRole('link', { name: 'Open Quarterly archive update from Alice Sender' });

    fireEvent.click(within(link).getByRole('checkbox', { name: 'Select Alice Sender' }));
    fireEvent.click(screen.getByRole('button', { name: 'Restore' }));
    await waitFor(() => expect(client.restoreCalls).toEqual(['thread/archive-row']));
    expect(client.classifyCalls).toEqual([]);

    fireEvent.click(within(link).getByRole('checkbox', { name: 'Deselect Alice Sender' }));
    fireEvent.click(within(link).getByRole('checkbox', { name: 'Select Alice Sender' }));
    fireEvent.click(screen.getByRole('button', { name: 'Delete forever' }));
    await waitFor(() => expect(client.destroyCalls).toEqual(['thread/archive-row']));
  });
});
