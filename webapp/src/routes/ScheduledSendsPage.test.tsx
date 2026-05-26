import { RouterProvider } from '@tanstack/react-router';
import { cleanup, fireEvent, screen, waitFor } from '@testing-library/react';
import type { ReactNode } from 'react';
import { afterEach, describe, expect, it } from 'vitest';
import type {
  CancelScheduledSendResponse,
  ScheduledSendsResponse,
} from '../api/client';
import { HailApiError } from '../api/client';
import { ApiClientProvider } from '../api/ApiClientProvider';
import { router } from '../router';
import {
  createTestQueryClient,
  renderWithQueryClient,
  seedMe,
  TestHailApiClient,
} from '../test-utils';
import { ScheduledSendsPage } from './ScheduledSendsPage';

class ScheduledSendsTestClient extends TestHailApiClient {
  readonly listCalls: string[] = [];
  readonly cancelCalls: number[] = [];
  cancelReject: Error | null = null;

  constructor(private sends: ScheduledSendsResponse) {
    super();
  }

  override async listScheduledSends(): Promise<ScheduledSendsResponse> {
    this.listCalls.push('list');
    return this.sends;
  }

  override async cancelScheduledSend(
    scheduledSendId: number,
  ): Promise<CancelScheduledSendResponse> {
    this.cancelCalls.push(scheduledSendId);
    if (this.cancelReject) {
      throw this.cancelReject;
    }

    const updated = this.sends.map((item) =>
      item.id === scheduledSendId
        ? { ...item, status: 'cancelled' }
        : item,
    );
    this.sends = updated;
    const item = updated.find((send) => send.id === scheduledSendId);
    if (!item) {
      throw new HailApiError(404, { error: 'not_found' }, response(404));
    }
    return item;
  }
}

afterEach(() => {
  currentTestBody = null;
  restoreRoute();
  window.history.pushState({}, '', '/');
  cleanup();
});

let currentTestBody: ReactNode = null;
let restoreScheduledRoute: (() => void) | null = null;

function restoreRoute() {
  restoreScheduledRoute?.();
  restoreScheduledRoute = null;
}

function TestBody() {
  return currentTestBody;
}

function installTestRouteComponent() {
  const matchRoute = router.routesByPath['/scheduled'];
  const previousComponent = matchRoute.options.component;
  const previousBeforeLoad = matchRoute.options.beforeLoad;
  matchRoute.options.component = TestBody;
  matchRoute.options.beforeLoad = undefined;
  restoreScheduledRoute = () => {
    matchRoute.options.component = previousComponent;
    matchRoute.options.beforeLoad = previousBeforeLoad;
  };
}

function renderScheduledSends(client: ScheduledSendsTestClient) {
  const queryClient = createTestQueryClient();
  seedMe(queryClient);
  currentTestBody = (
    <ApiClientProvider client={client}>
      <ScheduledSendsPage client={client} />
    </ApiClientProvider>
  );
  installTestRouteComponent();
  window.history.pushState({}, '', '/scheduled');

  renderWithQueryClient(<RouterProvider router={router} />, queryClient);
}

function scheduledSend(
  overrides: Partial<ScheduledSendsResponse[number]> = {},
): ScheduledSendsResponse[number] {
  return {
    id: 7,
    draft_email_id: 'draft-later-1',
    send_at: '2026-06-01T15:30:00Z',
    status: 'pending',
    created_at: '2026-05-26T12:00:00Z',
    claimed_at: null,
    sent_at: null,
    error: null,
    ...overrides,
  };
}

function response(status: number) {
  return new Response(JSON.stringify({}), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

describe('ScheduledSendsPage', () => {
  it('lists pending scheduled sends and cancels one', async () => {
    const client = new ScheduledSendsTestClient([
      scheduledSend(),
      scheduledSend({ id: 8, draft_email_id: 'draft-sent', status: 'sent' }),
    ]);

    renderScheduledSends(client);

    expect(await screen.findByText('Draft draft-later-1')).toBeInTheDocument();
    expect(screen.queryByText('Draft draft-sent')).not.toBeInTheDocument();
    expect(screen.getByText(/Sends at/)).toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));

    await waitFor(() => expect(client.cancelCalls).toEqual([7]));
    expect(await screen.findByText('Scheduled send cancelled.')).toBeInTheDocument();
    await waitFor(() =>
      expect(screen.getByText('No scheduled sends.')).toBeInTheDocument(),
    );
  });

  it('shows empty and error states', async () => {
    renderScheduledSends(new ScheduledSendsTestClient([]));

    expect(await screen.findByText('No scheduled sends.')).toBeInTheDocument();
    cleanup();

    const failingClient = new ScheduledSendsTestClient([]);
    failingClient.listScheduledSends = async () => {
      throw new HailApiError(500, { error: 'boom' }, response(500));
    };

    renderScheduledSends(failingClient);

    expect(await screen.findByText('Something went wrong.')).toBeInTheDocument();
    expect(
      screen.getByText('Scheduled sends failed with HTTP 500.'),
    ).toBeInTheDocument();
  });
});
