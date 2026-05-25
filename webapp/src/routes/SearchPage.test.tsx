import { RouterProvider } from '@tanstack/react-router';
import { cleanup, screen, within } from '@testing-library/react';
import type { ReactNode } from 'react';
import { afterEach, describe, expect, it } from 'vitest';
import type { SearchResponse } from '../api/client';
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
  override async search(): Promise<SearchResponse> {
    return { results: [] };
  }
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

  seedMe(queryClient);

  currentTestBody = (
    <AuthProvider>
      <SearchPage />
    </AuthProvider>
  );
  installTestRouteComponent();
  window.history.pushState({}, '', '/search');

  return renderWithQueryClient(
    <RouterProvider
      router={router}
      context={{ client: new SearchPageTestClient() }}
    />,
    queryClient,
  );
}

describe('SearchPage', () => {
  it('does not offer Clips scope while clips search is unsupported', async () => {
    renderSearchPage();

    const scope = await screen.findByLabelText('Scope');
    expect(scope).toHaveValue('all');
    expect(
      within(scope).getByRole('option', { name: 'All' }),
    ).toBeInTheDocument();
    expect(
      within(scope).getByRole('option', { name: 'Mail' }),
    ).toBeInTheDocument();
    expect(
      within(scope).getByRole('option', { name: 'Notes' }),
    ).toBeInTheDocument();
    expect(
      within(scope).queryByRole('option', { name: 'Clips' }),
    ).not.toBeInTheDocument();
    expect(
      screen.getByPlaceholderText('Search mail and notes'),
    ).toBeInTheDocument();
  });
});
