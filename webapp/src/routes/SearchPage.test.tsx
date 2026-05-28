import { RouterProvider } from '@tanstack/react-router';
import { cleanup, fireEvent, screen, waitFor } from '@testing-library/react';
import type { ReactNode } from 'react';
import { afterEach, describe, expect, it } from 'vitest';
import type { LabelListResponse, SearchParams, SearchResponse } from '../api/client';
import { ApiClientProvider } from '../api/ApiClientProvider';
import { AuthProvider } from '../auth/AuthProvider';
import { router } from '../router';
import {
  createTestQueryClient,
  renderWithQueryClient,
  seedMe,
  TestHailApiClient,
} from '../test-utils';
import { SearchPage } from './SearchPage';

class SearchPageTestClient extends TestHailApiClient {
  searchCalls: SearchParams[] = [];

  override async search(params: SearchParams): Promise<SearchResponse> {
    this.searchCalls.push(params);
    return { results: [] };
  }

  override async listLabels(): Promise<LabelListResponse> {
    return {
      labels: [
        {
          id: 10,
          name: 'Work',
          leaf_name: 'Work',
          path_segments: ['Work'],
          source: 'manual',
          color: null,
          thread_count: 2,
        },
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
    };
  }
}

if (!HTMLElement.prototype.hasPointerCapture) {
  HTMLElement.prototype.hasPointerCapture = () => false;
}
if (!HTMLElement.prototype.setPointerCapture) {
  HTMLElement.prototype.setPointerCapture = () => undefined;
}
if (!HTMLElement.prototype.scrollIntoView) {
  HTMLElement.prototype.scrollIntoView = () => undefined;
}

let currentTestBody: ReactNode = null;
let restoreSearchRoute: (() => void) | null = null;

afterEach(() => {
  currentTestBody = null;
  restoreSearchRoute?.();
  restoreSearchRoute = null;
  window.history.pushState({}, '', '/');
  cleanup();
});

function TestBody() {
  return currentTestBody;
}

function installTestRouteComponent() {
  const matchRoute = router.routesByPath['/search'];
  const previousComponent = matchRoute.options.component;
  const previousBeforeLoad = matchRoute.options.beforeLoad;
  matchRoute.options.component = TestBody;
  matchRoute.options.beforeLoad = undefined;
  restoreSearchRoute = () => {
    matchRoute.options.component = previousComponent;
    matchRoute.options.beforeLoad = previousBeforeLoad;
  };
}

function renderSearchPage() {
  const queryClient = createTestQueryClient();
  const client = new SearchPageTestClient();

  seedMe(queryClient);

  currentTestBody = (
    <ApiClientProvider client={client}>
      <AuthProvider>
        <SearchPage client={client} />
      </AuthProvider>
    </ApiClientProvider>
  );
  installTestRouteComponent();
  window.history.pushState({}, '', '/search');

  const view = renderWithQueryClient(
    <ApiClientProvider client={client}>
      <RouterProvider router={router} context={{ client }} />
    </ApiClientProvider>,
    queryClient,
  );

  return { ...view, client };
}

describe('SearchPage', () => {
  it('uses the centralized AppShell split container for search list plus reading content', async () => {
    renderSearchPage();

    await screen.findByLabelText('Label');
    const content = screen.getByTestId('app-shell-content');
    expect(content).toHaveAttribute('data-hail-content-layout', 'split');
    expect(content).toHaveClass('max-w-none', 'xl:max-w-7xl', 'min-w-0');
    expect(content.className).not.toContain('vw');
  });

  it('shows one concise help section before a search', async () => {
    renderSearchPage();

    await screen.findByLabelText('Label');

    expect(screen.getByText('Ready when you are')).toBeInTheDocument();
    expect(screen.getByText(/Search requires at least 2 characters/)).toBeInTheDocument();
    expect(screen.queryByText('Search mail and notes', { selector: 'p' })).not.toBeInTheDocument();
    expect(screen.queryByText(/Enter at least 2 characters/)).not.toBeInTheDocument();
  });

  it('defaults label filter to All and omits label_id from search calls', async () => {
    const { client } = renderSearchPage();

    expect(await screen.findByLabelText('Label')).toHaveTextContent('All');

    fireEvent.change(screen.getByPlaceholderText('Search mail and notes'), {
      target: { value: 'invoice' },
    });

    await waitFor(() => expect(client.searchCalls).toHaveLength(1));
    expect(client.searchCalls[0]).toEqual({
      q: 'invoice',
      scope: 'all',
      mailbox: 'all',
      label_id: undefined,
    });
  });

  it('AND-composes selected label with query and mailbox search filters', async () => {
    const { client } = renderSearchPage();

    fireEvent.pointerDown(await screen.findByLabelText('Mailbox'), {
      button: 0,
      ctrlKey: false,
      pointerType: 'mouse',
    });
    fireEvent.click(await screen.findByRole('option', { name: 'Paper Trail' }));
    fireEvent.pointerDown(screen.getByLabelText('Label'), {
      button: 0,
      ctrlKey: false,
      pointerType: 'mouse',
    });
    fireEvent.click(await screen.findByRole('option', { name: 'Work / Receipts' }));
    fireEvent.change(screen.getByPlaceholderText('Search mail and notes'), {
      target: { value: 'invoice' },
    });

    await waitFor(() => expect(client.searchCalls).toHaveLength(1));
    expect(client.searchCalls[0]).toEqual({
      q: 'invoice',
      scope: 'all',
      mailbox: 'papertrail',
      label_id: 12,
    });
  });
});
