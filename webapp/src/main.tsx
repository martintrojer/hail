import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { QueryClientProvider } from '@tanstack/react-query';
import {
  RouterProvider,
  createRootRoute,
  createRouter,
} from '@tanstack/react-router';
import App from './App';
import './index.css';
import { queryClient } from './lib/queryClient';

const rootRoute = createRootRoute({
  component: App,
});

const router = createRouter({
  routeTree: rootRoute,
});

declare module '@tanstack/react-router' {
  interface Register {
    router: typeof router;
  }
}

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <QueryClientProvider client={queryClient}>
      <RouterProvider router={router} />
    </QueryClientProvider>
  </StrictMode>,
);
