import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { QueryClientProvider } from '@tanstack/react-query';
import { RouterProvider } from '@tanstack/react-router';
import { HailApiClient } from './api/client';
import './index.css';
import { queryClient } from './lib/queryClient';
import { router } from './router';

const apiClient = new HailApiClient({ baseUrl: window.location.origin });

void apiClient
  .getHealthz()
  .then(() => {
    if (import.meta.env.DEV) {
      console.log('hail API health check passed');
    }
  })
  .catch((error: unknown) => {
    if (import.meta.env.DEV) {
      console.warn('hail API health check failed', error);
    }
  });

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <QueryClientProvider client={queryClient}>
      <RouterProvider router={router} />
    </QueryClientProvider>
  </StrictMode>,
);
