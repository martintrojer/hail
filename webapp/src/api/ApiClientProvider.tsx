import { createContext, useContext, type ReactNode } from 'react';
import type { HailApiClient } from './client';
import { defaultApiClient } from './query';

const ApiClientContext = createContext<HailApiClient>(defaultApiClient);

export function ApiClientProvider({
  client,
  children,
}: {
  client: HailApiClient;
  children: ReactNode;
}) {
  return (
    <ApiClientContext.Provider value={client}>{children}</ApiClientContext.Provider>
  );
}

export function useApiClient() {
  return useContext(ApiClientContext);
}
