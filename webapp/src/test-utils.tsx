import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { render } from '@testing-library/react';
import type { ReactElement, ReactNode } from 'react';
import { vi } from 'vitest';
import type { UserEnvelope } from './api/client';
import { HailApiClient } from './api/client';
import { queryKeys } from './api/queryKeys';
import type { router as appRouter } from './router';

export const defaultTestUser: UserEnvelope = {
  user: {
    id: 1,
    email: 'reader@example.com',
    display_name: 'Reader',
    is_admin: false,
  },
};

export class TestHailApiClient extends HailApiClient {
  constructor(readonly testUser: UserEnvelope = defaultTestUser) {
    super({ baseUrl: 'http://localhost' });
  }

  override async me(): Promise<UserEnvelope> {
    return this.testUser;
  }
}

export function createTestQueryClient() {
  return new QueryClient({
    defaultOptions: {
      queries: { retry: false },
      mutations: { retry: false },
    },
  });
}

export function seedMe(
  queryClient: QueryClient,
  user: UserEnvelope = defaultTestUser,
) {
  queryClient.setQueryData(queryKeys.me(), user);
}

export function renderWithQueryClient(
  ui: ReactElement,
  queryClient: QueryClient,
) {
  return render(
    <QueryClientProvider client={queryClient}>{ui}</QueryClientProvider>,
  );
}

export function withQueryClient(ui: ReactNode, queryClient: QueryClient) {
  return <QueryClientProvider client={queryClient}>{ui}</QueryClientProvider>;
}

type AppRouter = typeof appRouter;
type RouteByPath<Path extends keyof AppRouter['routesByPath']> =
  AppRouter['routesByPath'][Path];
type RouteOptions<Path extends keyof AppRouter['routesByPath']> =
  RouteByPath<Path>['options'];

export function installTestRoute<Path extends keyof AppRouter['routesByPath']>(
  router: AppRouter,
  path: Path,
  options: Partial<RouteOptions<Path>>,
) {
  const route = router.routesByPath[path];
  const previousOptions = { ...route.options };

  Object.assign(route.options, options);

  return () => {
    route.options = previousOptions;
  };
}

export function isolateAppQueryClientAuth(
  queryClient: QueryClient,
  user: UserEnvelope = defaultTestUser,
) {
  const previousDefaults = queryClient.getDefaultOptions();
  const previousMe = queryClient.getQueryData(queryKeys.me());

  seedMe(queryClient, user);
  queryClient.setDefaultOptions({
    ...previousDefaults,
    queries: {
      ...previousDefaults.queries,
      retry: false,
    },
  });

  return () => {
    if (previousMe === undefined) {
      queryClient.removeQueries({ queryKey: queryKeys.me() });
    } else {
      queryClient.setQueryData(queryKeys.me(), previousMe);
    }
    queryClient.setDefaultOptions(previousDefaults);
  };
}

export function installNoopHistoryBack() {
  return vi.spyOn(window.history, 'back').mockImplementation(() => undefined);
}

export function installNoNetworkFetch() {
  const fetchSpy = vi.spyOn(globalThis, 'fetch');

  return {
    fetchSpy,
    restore: () => fetchSpy.mockRestore(),
  };
}
