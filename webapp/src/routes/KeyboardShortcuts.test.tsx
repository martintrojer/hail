import { RouterProvider } from '@tanstack/react-router';
import { cleanup, fireEvent, screen, waitFor } from '@testing-library/react';
import type { ReactNode } from 'react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type {
  ImboxSectionedResponse,
  MailClassification,
  MailViewItem,
  ScreenerView,
  ThreadVerbResponse,
} from '../api/client';
import { AuthProvider } from '../auth/AuthProvider';
import { UndoToastProvider } from '../components/UndoToastProvider';
import { queryClient as appQueryClient } from '../lib/queryClient';
import { router } from '../router';
import {
  createTestQueryClient,
  renderWithQueryClient,
  seedMe,
  TestHailApiClient,
} from '../test-utils';
import { MailViewPage } from './MailViewPage';

vi.mock('../api/events', () => ({
  useHailEvents: vi.fn(),
}));

class KeyboardShortcutTestClient extends TestHailApiClient {
  readonly archiveCalls: string[] = [];
  readonly trashCalls: string[] = [];
  readonly setAsideCalls: string[] = [];
  readonly replyLaterCalls: string[] = [];
  readonly classifyCalls: Array<{ threadId: string; to: MailClassification }> = [];

  override async getImboxSectioned(): Promise<ImboxSectionedResponse> {
    return {
      bubbled_up: [],
      new_for_you: [
        mailItem({
          thread_id: 'thread-one',
          email_id: 'email-one',
          from: 'Alice Sender',
          subject: 'First shortcut thread',
          preview: 'Start here.',
        }),
        mailItem({
          thread_id: 'thread-two',
          email_id: 'email-two',
          from: 'Bob Sender',
          subject: 'Second shortcut thread',
          preview: 'Then here.',
        }),
      ],
      previously_seen: [
        mailItem({
          thread_id: 'thread-three',
          email_id: 'email-three',
          from: 'Cara Sender',
          subject: 'Previously seen shortcut thread',
          preview: 'Last item.',
        }),
      ],
      new_count: 2,
      previously_seen_total: 1,
    };
  }

  override async getScreenerView(): Promise<ScreenerView> {
    return { senders: [] };
  }

  override async archiveThread(threadId: string): Promise<ThreadVerbResponse> {
    this.archiveCalls.push(threadId);
    return threadVerbResponse('archive');
  }

  override async trashThread(threadId: string): Promise<ThreadVerbResponse> {
    this.trashCalls.push(threadId);
    return threadVerbResponse('trash');
  }

  override async setAsideThread(threadId: string): Promise<ThreadVerbResponse> {
    this.setAsideCalls.push(threadId);
    return threadVerbResponse('set-aside');
  }

  override async replyLaterThread(threadId: string): Promise<ThreadVerbResponse> {
    this.replyLaterCalls.push(threadId);
    return threadVerbResponse('reply-later');
  }

  override async classifyThread(
    threadId: string,
    to: MailClassification,
  ): Promise<ThreadVerbResponse> {
    this.classifyCalls.push({ threadId, to });
    return threadVerbResponse(`classify-${to}`);
  }
}

let currentTestBody: ReactNode = null;
let restoreImboxRoute: (() => void) | null = null;

function TestBody() {
  return currentTestBody;
}

function installTestRouteComponent() {
  const matchRoute = router.routesByPath['/imbox'];
  const previousComponent = matchRoute.options.component;
  const previousBeforeLoad = matchRoute.options.beforeLoad;
  matchRoute.options.component = TestBody;
  matchRoute.options.beforeLoad = undefined;
  restoreImboxRoute = () => {
    matchRoute.options.component = previousComponent;
    matchRoute.options.beforeLoad = previousBeforeLoad;
  };
}

afterEach(() => {
  currentTestBody = null;
  restoreImboxRoute?.();
  restoreImboxRoute = null;
  window.history.pushState({}, '', '/');
  cleanup();
  appQueryClient.clear();
  vi.restoreAllMocks();
});

beforeEach(() => {
  vi.spyOn(HTMLElement.prototype, 'offsetParent', 'get').mockImplementation(
    function offsetParent(this: HTMLElement) {
      return this.parentElement;
    },
  );
});

function renderKeyboardShortcutPage(client = new KeyboardShortcutTestClient()) {
  const queryClient = createTestQueryClient();
  seedMe(queryClient);
  seedMe(appQueryClient);
  currentTestBody = (
    <AuthProvider>
      <UndoToastProvider>
        <MailViewPage
          view="imbox"
          title="Imbox"
          description="Important mail from approved people lands here."
          client={client}
        />
      </UndoToastProvider>
    </AuthProvider>
  );
  installTestRouteComponent();
  window.history.pushState({}, '', '/imbox');

  renderWithQueryClient(<RouterProvider router={router} />, queryClient);

  return client;
}

function mailItem(overrides: Partial<MailViewItem>): MailViewItem {
  return {
    thread_id: 'thread-id',
    email_id: 'email-id',
    from: 'Sender',
    to: ['reader@example.com'],
    cc: [],
    bcc: [],
    subject: 'Shortcut thread',
    preview: 'Shortcut preview.',
    received_at: '2026-05-23T12:00:00Z',
    unread: true,
    classification: 'imbox',
    has_notes: false,
    labels: [],
    ...overrides,
  };
}

function threadVerbResponse(action: string): ThreadVerbResponse {
  return {
    undo: {
      id: `undo-${action}`,
      action: 'thread.stack',
      expires_at: '2026-05-23T13:00:00Z',
    },
  };
}

function press(key: string, init: Partial<KeyboardEvent> = {}) {
  fireEvent.keyDown(window, { key, ...init });
}

describe('SPA keyboard shortcuts', () => {
  it('focuses list rows with j/k/gg/G and opens the focused thread with o or Enter', async () => {
    renderKeyboardShortcutPage();

    const first = await screen.findByRole('link', {
      name: 'Open First shortcut thread from Alice Sender',
    });
    const second = screen.getByRole('link', {
      name: 'Open Second shortcut thread from Bob Sender',
    });
    const last = screen.getByRole('link', {
      name: 'Open Previously seen shortcut thread from Cara Sender',
    });

    press('j');
    expect(first).toHaveFocus();

    press('j');
    expect(second).toHaveFocus();

    press('k');
    expect(first).toHaveFocus();

    press('G');
    expect(last).toHaveFocus();

    press('g');
    press('g');
    expect(first).toHaveFocus();

    press('j');
    expect(second).toHaveFocus();
    press('o');
    await waitFor(() => expect(window.location.pathname).toBe('/thread/thread-two'));

    window.history.pushState({}, '', '/imbox');
    await router.invalidate();
    const reloadedFirst = await screen.findByRole('link', {
      name: 'Open First shortcut thread from Alice Sender',
    });
    press('j');
    expect(reloadedFirst).toHaveFocus();
    press('Enter');
    await waitFor(() => expect(window.location.pathname).toBe('/thread/thread-one'));
  });

  it('navigates with g-prefix shortcuts, compose, and search focus', async () => {
    renderKeyboardShortcutPage();

    await screen.findByRole('heading', { name: 'Imbox' });

    press('g');
    press('f');
    await waitFor(() => expect(window.location.pathname).toBe('/feed'));

    window.history.pushState({}, '', '/imbox');
    await router.invalidate();
    await screen.findByRole('heading', { name: 'Imbox' });

    press('g');
    press('t');
    await waitFor(() => expect(window.location.pathname).toBe('/trash'));

    window.history.pushState({}, '', '/imbox');
    await router.invalidate();
    await screen.findByRole('heading', { name: 'Imbox' });

    press('c');
    await waitFor(() => expect(window.location.pathname).toBe('/compose'));

    window.history.pushState({}, '', '/imbox');
    await router.invalidate();
    await screen.findByRole('heading', { name: 'Imbox' });

    press('/');
    await waitFor(() => expect(window.location.pathname).toBe('/search'));
    const searchInput = await screen.findByPlaceholderText('Search mail and notes');
    await waitFor(() => expect(searchInput).toHaveFocus());
  });

  it('opens and closes the help overlay without leaking Escape to the shell handler', async () => {
    renderKeyboardShortcutPage();

    await screen.findByRole('heading', { name: 'Imbox' });

    press('?');
    const dialog = await screen.findByRole('dialog', { name: 'Keyboard Shortcuts' });
    expect(dialog).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Close' })).toHaveFocus();

    press('Escape');
    await waitFor(() => {
      expect(screen.queryByRole('dialog', { name: 'Keyboard Shortcuts' })).not.toBeInTheDocument();
    });
  });

  it('routes focused list action shortcuts to the focused thread and ignores editable targets', async () => {
    const client = renderKeyboardShortcutPage();

    const first = await screen.findByRole('link', {
      name: 'Open First shortcut thread from Alice Sender',
    });
    const second = screen.getByRole('link', {
      name: 'Open Second shortcut thread from Bob Sender',
    });

    press('j');
    expect(first).toHaveFocus();
    press('j');
    expect(second).toHaveFocus();

    press('e');
    await waitFor(() => expect(client.archiveCalls).toEqual(['thread-two']));
    expect(await screen.findByText('Thread archived.')).toBeInTheDocument();

    press('y');
    await waitFor(() => expect(client.setAsideCalls).toEqual(['thread-two']));

    const editableTarget = document.createElement('input');
    document.body.append(editableTarget);
    editableTarget.focus();
    fireEvent.keyDown(editableTarget, { key: 'd' });
    expect(client.trashCalls).toEqual([]);
    editableTarget.remove();

    second.focus();
    press('d');
    await waitFor(() => expect(client.trashCalls).toEqual(['thread-two']));

    press('l');
    await waitFor(() => expect(client.replyLaterCalls).toEqual(['thread-two']));
  });
});
