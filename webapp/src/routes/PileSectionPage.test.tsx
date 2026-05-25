import { RouterProvider } from '@tanstack/react-router';
import { QueryClient } from '@tanstack/react-query';
import {
  cleanup,
  fireEvent,
  screen,
  waitFor,
} from '@testing-library/react';
import type { ReactNode } from 'react';
import { afterEach, describe, expect, it } from 'vitest';
import type {
  ComposeResponse,
  PileViewResponse,
  ReplyRequest,
  ThreadVerbResponse,
} from '../api/client';
import {
  createTestQueryClient,
  renderWithQueryClient,
  seedMe,
  TestHailApiClient,
} from '../test-utils';
import { PileSectionPage } from './PileSectionPage';
import { defaultApiClient } from '../api/query';
import { router } from '../router';

class PileSectionPageTestClient extends TestHailApiClient {
  readonly sendReplyCalls: Array<{ threadId: string; body: ReplyRequest }> = [];
  readonly classifyThreadCalls: Array<{ threadId: string; to: 'imbox' | 'feed' | 'papertrail' }> = [];
  replyLaterResponse: PileViewResponse = {
    items: [
      {
        thread_id: 'thread-1',
        position: 1,
        added_at: '2026-05-25T12:00:00Z',
        preview: {
          sender: 'Alice',
          subject: 'Launch plan',
          snippet: 'Can you review this?',
        },
      },
    ],
  };
  classifyError: unknown;

  override async getReplyLater(): Promise<PileViewResponse> {
    return this.replyLaterResponse;
  }

  override async sendReply(
    threadId: string,
    body: ReplyRequest,
  ): Promise<ComposeResponse> {
    this.sendReplyCalls.push({ threadId, body });
    return {
      status: 'sent',
      email_id: 'reply-email-1',
      submission_id: 'reply-submission-1',
    };
  }

  override async classifyThread(
    threadId: string,
    to: 'imbox' | 'feed' | 'papertrail',
  ): Promise<ThreadVerbResponse> {
    this.classifyThreadCalls.push({ threadId, to });
    if (this.classifyError) throw this.classifyError;
    return {};
  }
}

let currentTestBody: ReactNode = null;
let restoreReplyLaterRoute: (() => void) | null = null;
const originalDefaultMe = defaultApiClient.me;
const originalDefaultGetReplyLater = defaultApiClient.getReplyLater;
const originalDefaultSendReply = defaultApiClient.sendReply;
const originalDefaultClassifyThread = defaultApiClient.classifyThread;

function TestBody() {
  return currentTestBody;
}

function installTestRouteComponent() {
  const matchRoute = router.routesByPath['/reply-later'];
  const previousComponent = matchRoute.options.component;
  const previousBeforeLoad = matchRoute.options.beforeLoad;
  matchRoute.options.component = TestBody;
  matchRoute.options.beforeLoad = undefined;
  restoreReplyLaterRoute = () => {
    matchRoute.options.component = previousComponent;
    matchRoute.options.beforeLoad = previousBeforeLoad;
  };
}

function patchDefaultClient(client: PileSectionPageTestClient) {
  defaultApiClient.me = client.me.bind(client);
  defaultApiClient.getReplyLater = client.getReplyLater.bind(client);
  defaultApiClient.sendReply = client.sendReply.bind(client);
  defaultApiClient.classifyThread = client.classifyThread.bind(client);
}

function renderReplyLaterPage(client = new PileSectionPageTestClient()) {
  const queryClient: QueryClient = createTestQueryClient();
  seedMe(queryClient, client.testUser);

  currentTestBody = <PileSectionPage kind="reply-later" />;
  installTestRouteComponent();
  patchDefaultClient(client);
  window.history.pushState({}, '', '/reply-later');

  renderWithQueryClient(<RouterProvider router={router} />, queryClient);

  return client;
}


afterEach(() => {
  defaultApiClient.me = originalDefaultMe;
  defaultApiClient.getReplyLater = originalDefaultGetReplyLater;
  defaultApiClient.sendReply = originalDefaultSendReply;
  defaultApiClient.classifyThread = originalDefaultClassifyThread;
  currentTestBody = null;
  restoreReplyLaterRoute?.();
  restoreReplyLaterRoute = null;
  window.history.pushState({}, '', '/');
  cleanup();
});

describe('PileSectionPage Reply Later panel', () => {
  it('sends replies through the shared mutation and moves the thread to Imbox', async () => {
    const client = renderReplyLaterPage();

    fireEvent.click(await screen.findByRole('button', { name: /Select Launch plan from Alice to reply/ }));
    fireEvent.change(screen.getByPlaceholderText('Write your reply…'), {
      target: { value: 'I will review this today.' },
    });
    fireEvent.click(screen.getByRole('button', { name: 'Reply' }));

    await waitFor(() => expect(client.sendReplyCalls).toHaveLength(1));
    expect(client.sendReplyCalls[0]).toEqual({
      threadId: 'thread-1',
      body: {
        body_markdown: 'I will review this today.',
        attachments: [],
        send_at: undefined,
      },
    });
    await waitFor(() => expect(client.classifyThreadCalls).toEqual([
      { threadId: 'thread-1', to: 'imbox' },
    ]));
    expect(await screen.findByText('Reply sent ✓')).toBeInTheDocument();
  });

  it('surfaces a post-send move failure instead of hiding it', async () => {
    const client = new PileSectionPageTestClient();
    client.classifyError = new Error('move failed');
    renderReplyLaterPage(client);

    fireEvent.click(await screen.findByRole('button', { name: /Select Launch plan from Alice to reply/ }));
    fireEvent.change(screen.getByPlaceholderText('Write your reply…'), {
      target: { value: 'Reply body.' },
    });
    fireEvent.click(screen.getByRole('button', { name: 'Reply' }));

    await waitFor(() => expect(client.sendReplyCalls).toHaveLength(1));
    expect(
      await screen.findByText('Reply sent, but moving it back to Imbox failed. Try moving it manually.'),
    ).toBeInTheDocument();
    expect(screen.queryByText('Reply sent ✓')).not.toBeInTheDocument();
  });
});
