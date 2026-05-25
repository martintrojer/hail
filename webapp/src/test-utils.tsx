import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { render } from '@testing-library/react';
import type { ReactElement, ReactNode } from 'react';
import type { UserEnvelope } from './api/client';
import { HailApiClient } from './api/client';
import { queryKeys } from './api/queryKeys';

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
