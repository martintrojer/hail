import type { ReactNode } from 'react';
import { cleanup, screen, waitFor, within } from '@testing-library/react';
import { RouterProvider } from '@tanstack/react-router';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { LabelThreadsResponse } from '../api/client';
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
import { LabelViewPage } from './LabelViewPage';

class LabelViewPageTestClient extends TestHailApiClient {
  readonly labelThreadCalls: Array<{ labelId: number; cursor?: string }> = [];

  constructor(private readonly responses: Promise<LabelThreadsResponse>[]) {
    super();
  }

  override async getLabelThreads(
    labelId: number,
    params: { cursor?: string; limit?: number } = {},
  ): Promise<LabelThreadsResponse> {
    this.labelThreadCalls.push({ labelId, cursor: params.cursor });
    const response = this.responses.shift();
    if (!response) {
      throw new Error(`Unexpected label thread request for cursor ${params.cursor ?? 'initial'}`);
    }
    return response;
  }
}

let currentTestBody: ReactNode = null;
let restoreLabelRoute: (() => void) | null = null;

function TestBody() {
  return currentTestBody;
}

function installTestRouteComponent() {
  const matchRoute = router.routesByPath['/labels/$labelId'];
  const previousComponent = matchRoute.options.component;
  const previousBeforeLoad = matchRoute.options.beforeLoad;
  matchRoute.options.component = TestBody;
  matchRoute.options.beforeLoad = undefined;
  restoreLabelRoute = () => {
    matchRoute.options.component = previousComponent;
    matchRoute.options.beforeLoad = previousBeforeLoad;
  };
}

function restoreRoute() {
  restoreLabelRoute?.();
  restoreLabelRoute = null;
}

afterEach(() => {
  currentTestBody = null;
  restoreRoute();
  window.history.pushState({}, '', '/');
  cleanup();
});

function renderLabelView(labelId: number, client: LabelViewPageTestClient) {
  const queryClient = createTestQueryClient();
  seedMe(queryClient);

  currentTestBody = (
    <AuthProvider>
      <UndoToastProvider>
        <LabelViewPage labelId={labelId} client={client} />
      </UndoToastProvider>
    </AuthProvider>
  );
  installTestRouteComponent();
  window.history.pushState({}, '', `/labels/${labelId}`);

  renderWithQueryClient(<RouterProvider router={router} />, queryClient);

  return client;
}

function labelThreadsResponse(overrides: Partial<LabelThreadsResponse> = {}): LabelThreadsResponse {
  return {
    label: {
      id: 42,
      name: 'Work/Receipts',
      leaf_name: 'Receipts',
      path_segments: ['Work', 'Receipts'],
      source: 'manual',
      color: null,
      thread_count: 1,
    },
    items: [
      {
        thread_id: 'thread-1',
        from: 'Alice Sender',
        subject: 'Invoice update',
        preview: 'Your invoice is ready.',
        labels: [],
      },
    ],
    next_cursor: null,
    ...overrides,
  };
}

function response(status: number, body: unknown = {}) {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

describe('LabelViewPage', () => {
  it('renders the label path header and assigned threads', async () => {
    const client = renderLabelView(
      42,
      new LabelViewPageTestClient([Promise.resolve(labelThreadsResponse())]),
    );

    expect(await screen.findByRole('heading', { name: 'Work / Receipts' })).toBeInTheDocument();
    expect(screen.getByText('1 thread')).toBeInTheDocument();
    const thread = screen.getByRole('link', {
      name: 'Open Invoice update from Alice Sender',
    });
    expect(within(thread).getByText('Alice Sender')).toBeInTheDocument();
    expect(within(thread).getByText('Invoice update')).toBeInTheDocument();
    expect(within(thread).getByText('Your invoice is ready.')).toBeInTheDocument();
    expect(client.labelThreadCalls).toEqual([{ labelId: 42, cursor: undefined }]);
  });

  it('fetches the next cursor page and renders accumulated label threads', async () => {
    const observerInstances: Array<{
      callback: IntersectionObserverCallback;
      observed: Element[];
      disconnect: ReturnType<typeof vi.fn>;
    }> = [];
    const originalObserver = window.IntersectionObserver;
    window.IntersectionObserver = vi.fn(function MockIntersectionObserver(
      this: IntersectionObserver,
      callback: IntersectionObserverCallback,
    ) {
      const instance = {
        callback,
        observed: [] as Element[],
        disconnect: vi.fn(),
      };
      observerInstances.push(instance);
      this.observe = (element: Element) => instance.observed.push(element);
      this.unobserve = vi.fn();
      this.disconnect = instance.disconnect;
      this.takeRecords = () => [];
      Object.defineProperties(this, {
        root: { value: null },
        rootMargin: { value: '' },
        thresholds: { value: [] },
      });
    }) as unknown as typeof IntersectionObserver;

    try {
      const client = renderLabelView(
        42,
        new LabelViewPageTestClient([
          Promise.resolve(labelThreadsResponse({ next_cursor: '1' })),
          Promise.resolve(
            labelThreadsResponse({
              items: [
                {
                  thread_id: 'thread-2',
                  from: 'Bob Sender',
                  subject: 'Second invoice',
                  preview: 'Another invoice is ready.',
                  labels: [],
                },
              ],
              next_cursor: null,
            }),
          ),
        ]),
      );

      expect(await screen.findByRole('link', {
        name: 'Open Invoice update from Alice Sender',
      })).toBeInTheDocument();
      await waitFor(() => expect(observerInstances[0]?.observed.length).toBe(1));

      observerInstances[0].callback(
        [
          {
            target: observerInstances[0].observed[0],
            isIntersecting: true,
          } as IntersectionObserverEntry,
        ],
        {} as IntersectionObserver,
      );

      expect(await screen.findByRole('link', {
        name: 'Open Second invoice from Bob Sender',
      })).toBeInTheDocument();
      expect(screen.getByRole('link', {
        name: 'Open Invoice update from Alice Sender',
      })).toBeInTheDocument();
      expect(screen.getByText("You're all caught up")).toBeInTheDocument();
      expect(client.labelThreadCalls).toEqual([
        { labelId: 42, cursor: undefined },
        { labelId: 42, cursor: '1' },
      ]);
    } finally {
      window.IntersectionObserver = originalObserver;
    }
  });

  it('renders hydrated label chips for multi-label rows', async () => {
    renderLabelView(
      42,
      new LabelViewPageTestClient([
        Promise.resolve(
          labelThreadsResponse({
            items: [
              {
                thread_id: 'thread-1',
                from: 'Alice Sender',
                subject: 'Invoice update',
                preview: 'Your invoice is ready.',
                labels: [
                  {
                    id: 42,
                    name: 'Work/Receipts',
                    leaf_name: 'Receipts',
                    path_segments: ['Work', 'Receipts'],
                    source: 'manual',
                    color: null,
                    thread_count: 2,
                  },
                  {
                    id: 99,
                    name: 'People/Alice',
                    leaf_name: 'Alice',
                    path_segments: ['People', 'Alice'],
                    source: 'manual',
                    color: null,
                    thread_count: 3,
                  },
                ],
              },
            ],
          }),
        ),
      ]),
    );

    const thread = await screen.findByRole('link', {
      name: 'Open Invoice update from Alice Sender',
    });
    expect(within(thread).getByLabelText('Label Work/Receipts')).toBeInTheDocument();
    expect(within(thread).getByText('Receipts')).toHaveAttribute('title', 'Work/Receipts');
    expect(within(thread).getByLabelText('Label People/Alice')).toBeInTheDocument();
    expect(within(thread).getByText('Alice')).toHaveAttribute('title', 'People/Alice');
  });

  it('renders an empty state for labels without assigned threads', async () => {
    renderLabelView(
      42,
      new LabelViewPageTestClient([Promise.resolve(labelThreadsResponse({ items: [] }))]),
    );

    expect(await screen.findByRole('heading', { name: 'Work / Receipts' })).toBeInTheDocument();
    expect(screen.getByText('No mail with this label yet.')).toBeInTheDocument();
  });

  it('renders not found errors from the label thread API', async () => {
    renderLabelView(
      42,
      new LabelViewPageTestClient([
        Promise.reject(new HailApiError(404, {}, response(404))),
      ]),
    );

    expect(await screen.findByText('Something went wrong.')).toBeInTheDocument();
    expect(
      screen.getByText('This label was not found. It may have been renamed or deleted.'),
    ).toBeInTheDocument();
  });
});
