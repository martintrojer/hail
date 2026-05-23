export const queryKeys = {
  all: ['hail'] as const,
  auth: () => [...queryKeys.all, 'auth'] as const,
  me: () => [...queryKeys.auth(), 'me'] as const,
  setup: () => [...queryKeys.all, 'setup'] as const,
  setupState: () => [...queryKeys.setup(), 'state'] as const,
  views: () => [...queryKeys.all, 'views'] as const,
  view: (view: 'imbox' | 'feed' | 'papertrail' | 'set-aside' | 'reply-later') =>
    [...queryKeys.views(), view] as const,
  search: (q: string, scope: 'all' | 'mail' | 'notes' | 'clips') =>
    [...queryKeys.views(), 'search', scope, q] as const,
  screener: () => [...queryKeys.views(), 'screener'] as const,
  contacts: () => [...queryKeys.all, 'contacts'] as const,
  contact: (address: string) =>
    [...queryKeys.contacts(), address.trim().toLowerCase()] as const,
  threads: () => [...queryKeys.all, 'threads'] as const,
  thread: (threadId: string) => [...queryKeys.threads(), threadId] as const,
};
