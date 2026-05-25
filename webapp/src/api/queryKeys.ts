export const queryKeys = {
  all: ['hail'] as const,
  auth: () => [...queryKeys.all, 'auth'] as const,
  me: () => [...queryKeys.auth(), 'me'] as const,
  setup: () => [...queryKeys.all, 'setup'] as const,
  setupState: () => [...queryKeys.setup(), 'state'] as const,
  admin: () => [...queryKeys.all, 'admin'] as const,
  adminUsers: () => [...queryKeys.admin(), 'users'] as const,
  adminDomains: () => [...queryKeys.admin(), 'domains'] as const,
  views: () => [...queryKeys.all, 'views'] as const,
  view: (view: 'imbox' | 'feed' | 'papertrail' | 'drafts' | 'set-aside' | 'reply-later' | 'bubble-up') =>
    [...queryKeys.views(), view] as const,
  search: (q: string, scope: 'all' | 'mail' | 'notes' | 'clips') =>
    [...queryKeys.views(), 'search', scope, q] as const,
  screener: () => [...queryKeys.views(), 'screener'] as const,
  screenerDenied: () => [...queryKeys.screener(), 'denied'] as const,
  contacts: () => [...queryKeys.all, 'contacts'] as const,
  contact: (address: string) =>
    [...queryKeys.contacts(), address.trim().toLowerCase()] as const,
  threads: () => [...queryKeys.all, 'threads'] as const,
  thread: (threadId: string) => [...queryKeys.threads(), threadId] as const,
};
