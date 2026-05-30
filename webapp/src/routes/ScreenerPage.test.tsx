import { RouterProvider } from '@tanstack/react-router';
import { cleanup, fireEvent, screen, waitFor } from '@testing-library/react';
import type { ReactNode } from 'react';
import { afterEach, describe, expect, it } from 'vitest';
import type {
  ScreenerDecisionRequest,
  ScreenerDecisionResponse,
  ScreenerView,
} from '../api/client';
import { HailApiError } from '../api/client';
import { AuthProvider } from '../auth/AuthProvider';
import { UndoToastProvider } from '../components/UndoToastProvider';
import { router } from '../router';
import {
  createTestQueryClient,
  renderWithQueryClient,
  seedMe,
  TestHailApiClient,
} from '../test-utils';
import { ScreenerPage } from './ScreenerPage';

class ScreenerPageTestClient extends TestHailApiClient {
  readonly decideScreenerCalls: ScreenerDecisionRequest[] = [];
  private viewPromise: Promise<ScreenerView>;
  private decisionHandler: (
    body: ScreenerDecisionRequest,
  ) => Promise<ScreenerDecisionResponse>;

  constructor({
    view = sampleScreenerView(),
    viewPromise,
    decisionHandler,
  }: {
    view?: ScreenerView;
    viewPromise?: Promise<ScreenerView>;
    decisionHandler?: (
      body: ScreenerDecisionRequest,
    ) => Promise<ScreenerDecisionResponse>;
  } = {}) {
    super({
      user: {
        id: 1,
        email: 'screener@example.com',
        display_name: 'Screener',
        is_admin: false,
      },
    });
    this.viewPromise = viewPromise ?? Promise.resolve(view);
    this.decisionHandler =
      decisionHandler ??
      ((body) =>
        Promise.resolve({
          sender: body.sender,
          decision: body.decision,
          classify_as:
            body.classify_as === 'imbox' ||
            body.classify_as === 'feed' ||
            body.classify_as === 'papertrail'
              ? body.classify_as
              : null,
        }));
  }

  override async getScreenerView(): Promise<ScreenerView> {
    return this.viewPromise;
  }

  override async decideScreener(
    body: ScreenerDecisionRequest,
  ): Promise<ScreenerDecisionResponse> {
    this.decideScreenerCalls.push(body);
    return this.decisionHandler(body);
  }

}

let currentTestBody: ReactNode = null;
let restoreScreenerRoute: (() => void) | null = null;

function restoreRoute() {
  restoreScreenerRoute?.();
  restoreScreenerRoute = null;
}

afterEach(() => {
  currentTestBody = null;
  restoreRoute();
  window.history.pushState({}, '', '/');
  cleanup();
});

function TestBody() {
  return currentTestBody;
}

function installTestRouteComponent() {
  const matchRoute = router.routesByPath['/screener'];
  const previousComponent = matchRoute.options.component;
  const previousBeforeLoad = matchRoute.options.beforeLoad;
  matchRoute.options.component = TestBody;
  matchRoute.options.beforeLoad = undefined;
  restoreScreenerRoute = () => {
    matchRoute.options.component = previousComponent;
    matchRoute.options.beforeLoad = previousBeforeLoad;
  };
}

function renderScreener(client = new ScreenerPageTestClient()) {
  const queryClient = createTestQueryClient();

  seedMe(queryClient, client.testUser);

  currentTestBody = (
    <AuthProvider>
      <UndoToastProvider>
        <ScreenerPage client={client} />
      </UndoToastProvider>
    </AuthProvider>
  );
  installTestRouteComponent();
  window.history.pushState({}, '', '/screener');

  renderWithQueryClient(<RouterProvider router={router} />, queryClient);

  return client;
}

function sampleScreenerView(
  overrides: Partial<ScreenerView> = {},
): ScreenerView {
  return {
    senders: [
      {
        sender: 'newsletter@example.com',
        first_seen_at: '2026-05-23T12:00:00Z',
        message_count: 3,
        latest_preview: {
          from: 'newsletter@example.com',
          subject: 'Newsletter dispatch',
          preview: 'Latest dispatch from the newsletter.',
          received_at: '2026-05-23T12:15:00Z',
        },
        emails: [
          {
            email_id: 'email-3',
            subject: 'Newsletter dispatch',
            preview: 'Latest dispatch from the newsletter.',
            received_at: '2026-05-23T12:15:00Z',
          },
          {
            email_id: 'email-2',
            subject: 'Earlier dispatch',
            preview: 'Earlier newsletter issue.',
            received_at: '2026-05-22T12:15:00Z',
          },
          {
            email_id: 'email-1',
            subject: '',
            preview: '',
            received_at: null,
          },
        ],
      },
    ],
    ...overrides,
  };
}

function response(status: number, body: unknown = {}) {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

function openDropdown(button: HTMLElement) {
  fireEvent.pointerDown(button, {
    ctrlKey: false,
    button: 0,
  });
}

describe('ScreenerPage', () => {
  it('expands a sender card to show all pending emails', async () => {
    renderScreener();

    expect(await screen.findByText('Newsletter dispatch')).toBeInTheDocument();
    expect(screen.queryByText('Earlier dispatch')).not.toBeInTheDocument();

    fireEvent.click(
      screen.getByRole('button', { name: /Show · 3 pending emails/i }),
    );

    expect(
      screen.getByRole('button', { name: /Hide · 3 pending emails/i }),
    ).toHaveAttribute('aria-expanded', 'true');
    expect(screen.getByText('Earlier dispatch')).toBeInTheDocument();
    expect(screen.getByText('Earlier newsletter issue.')).toBeInTheDocument();
    expect(screen.getByText('May 22, 2026')).toBeInTheDocument();
    expect(screen.getByText('No subject')).toBeInTheDocument();
    expect(screen.queryByText('Preview unavailable.')).not.toBeInTheDocument();
    expect(screen.getByText('Date unavailable')).toBeInTheDocument();
  });

  it('hides empty preview lines instead of rendering unavailable placeholder copy', async () => {
    renderScreener(
      new ScreenerPageTestClient({
        view: sampleScreenerView({
          senders: [
            {
              sender: 'blank@example.com',
              first_seen_at: '2026-05-23T12:00:00Z',
              message_count: 1,
              latest_preview: {
                from: 'blank@example.com',
                subject: 'Blank preview subject',
                preview: '',
                received_at: '2026-05-23T12:15:00Z',
              },
              emails: [
                {
                  email_id: 'blank-email-1',
                  subject: 'Blank child subject',
                  preview: '',
                  received_at: '2026-05-23T12:15:00Z',
                },
              ],
            },
          ],
        }),
      }),
    );

    expect(await screen.findByText('Blank preview subject')).toBeInTheDocument();
    expect(screen.queryByText(/Preview unavailable/i)).not.toBeInTheDocument();

    fireEvent.click(
      screen.getByRole('button', { name: /Show · 1 pending email/i }),
    );

    expect(screen.getByText('Blank child subject')).toBeInTheDocument();
    expect(screen.queryByText(/Preview unavailable/i)).not.toBeInTheDocument();
  });

  it('renders non-empty latest and email previews', async () => {
    renderScreener(
      new ScreenerPageTestClient({
        view: sampleScreenerView({
          senders: [
            {
              sender: 'preview@example.com',
              first_seen_at: '2026-05-23T12:00:00Z',
              message_count: 1,
              latest_preview: {
                from: 'preview@example.com',
                subject: 'Preview subject',
                preview: 'Visible latest body excerpt.',
                received_at: '2026-05-23T12:15:00Z',
              },
              emails: [
                {
                  email_id: 'preview-email-1',
                  subject: 'Preview child subject',
                  preview: 'Visible child body excerpt.',
                  received_at: '2026-05-23T12:15:00Z',
                },
              ],
            },
          ],
        }),
      }),
    );

    expect(await screen.findByText('Visible latest body excerpt.')).toBeInTheDocument();

    fireEvent.click(
      screen.getByRole('button', { name: /Show · 1 pending email/i }),
    );

    expect(screen.getByText('Visible child body excerpt.')).toBeInTheDocument();
  });

  it('opens routing choices before approving a sender with history backfill', async () => {
    const client = renderScreener();

    openDropdown(await screen.findByRole('button', { name: 'Approve' }));
    expect(screen.getByRole('menu')).toBeInTheDocument();
    const imbox = screen.getByRole('menuitemradio', { name: 'The Imbox' });
    expect(imbox).toHaveAttribute('aria-checked', 'true');
    expect(screen.getByRole('menuitemradio', { name: 'The Feed' })).toHaveAttribute(
      'aria-checked',
      'false',
    );

    fireEvent.click(screen.getByRole('menuitemradio', { name: 'The Feed' }));

    await waitFor(() => expect(client.decideScreenerCalls).toHaveLength(1));
    expect(client.decideScreenerCalls[0]).toEqual({
      sender: 'newsletter@example.com',
      decision: 'approve',
      classify_as: 'feed',
      apply_to_history: true,
    });
  });

  it('denies a sender without classification and applies the decision to history', async () => {
    const client = renderScreener();

    fireEvent.click(await screen.findByRole('button', { name: 'Deny' }));

    await waitFor(() => expect(client.decideScreenerCalls).toHaveLength(1));
    expect(client.decideScreenerCalls[0]).toEqual({
      sender: 'newsletter@example.com',
      decision: 'deny',
      apply_to_history: true,
    });
  });

  it('shows initial pending, error, and empty states', async () => {
    const neverResolves = new Promise<ScreenerView>(() => undefined);
    const pendingClient = renderScreener(
      new ScreenerPageTestClient({ viewPromise: neverResolves }),
    );

    expect(
      screen.getByLabelText('Loading pending senders'),
    ).toBeInTheDocument();
    expect(pendingClient.decideScreenerCalls).toEqual([]);
    cleanup();
    restoreRoute();

    renderScreener(
      new ScreenerPageTestClient({
        viewPromise: Promise.reject(new HailApiError(401, {}, response(401))),
      }),
    );
    expect(
      await screen.findByText('Something went wrong.'),
    ).toBeInTheDocument();
    expect(
      screen.getByText(
        'Your session expired. Sign in again to refresh the Screener.',
      ),
    ).toBeInTheDocument();
    cleanup();
    restoreRoute();

    renderScreener(
      new ScreenerPageTestClient({ view: sampleScreenerView({ senders: [] }) }),
    );
    expect(await screen.findByText('No unknown senders')).toBeInTheDocument();
  });

  it('disables card controls while a decision is pending', async () => {
    const decisionPromise = new Promise<ScreenerDecisionResponse>(
      () => undefined,
    );
    const client = renderScreener(
      new ScreenerPageTestClient({
        decisionHandler: () => decisionPromise,
      }),
    );

    fireEvent.click(await screen.findByRole('button', { name: 'Deny' }));

    await waitFor(() => expect(client.decideScreenerCalls).toHaveLength(1));
    expect(screen.getByRole('button', { name: 'Saving…' })).toBeDisabled();
    expect(screen.getByRole('button', { name: 'Deny' })).toBeDisabled();
  });

  it('shows a decision error after choosing a routing destination', async () => {
    const client = renderScreener(
      new ScreenerPageTestClient({
        decisionHandler: () =>
          Promise.reject(new HailApiError(422, {}, response(422))),
      }),
    );

    openDropdown(await screen.findByRole('button', { name: 'Approve' }));
    fireEvent.click(screen.getByRole('menuitemradio', { name: 'Paper Trail' }));

    expect(
      await screen.findByRole('alert', {
        name: '',
      }),
    ).toHaveTextContent(
      'The server rejected this decision. Refresh and try again.',
    );
    expect(client.decideScreenerCalls[0]).toMatchObject({
      classify_as: 'papertrail',
      apply_to_history: true,
    });
  });

  it('shows an undo toast for denied senders when the API returns an undo token', async () => {
    const client = renderScreener(
      new ScreenerPageTestClient({
        decisionHandler: (body) =>
          Promise.resolve({
            sender: body.sender,
            decision: 'deny',
            undo: {
              id: 'undo-deny-1',
              action: 'screener.deny',
              expires_at: '2026-05-23T12:05:00Z',
            },
          }),
      }),
    );

    fireEvent.click(await screen.findByRole('button', { name: 'Deny' }));

    expect(
      await screen.findByText('Denied newsletter@example.com.'),
    ).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Undo' })).toBeInTheDocument();
    expect(client.decideScreenerCalls).toHaveLength(1);
  });
});
