export const queryKeys = {
  all: ['hail'] as const,
  auth: () => [...queryKeys.all, 'auth'] as const,
  me: () => [...queryKeys.auth(), 'me'] as const,
  setup: () => [...queryKeys.all, 'setup'] as const,
  setupState: () => [...queryKeys.setup(), 'state'] as const,
  invite: (token: string) => [...queryKeys.all, 'invite', token] as const,
  admin: () => [...queryKeys.all, 'admin'] as const,
  adminUsers: () => [...queryKeys.admin(), 'users'] as const,
  adminStats: () => [...queryKeys.admin(), 'stats'] as const,
  adminDomains: () => [...queryKeys.admin(), 'domains'] as const,
  views: () => [...queryKeys.all, 'views'] as const,
  viewCounts: () => [...queryKeys.views(), 'counts'] as const,
  attachments: () => [...queryKeys.all, 'attachments'] as const,
  view: (view: 'imbox' | 'feed' | 'papertrail' | 'drafts' | 'trash' | 'spam' | 'archive' | 'set-aside' | 'reply-later' | 'bubble-up') =>
    [...queryKeys.views(), view] as const,
  imboxSectioned: () => [...queryKeys.view('imbox'), 'sectioned'] as const,
  labelThreadsRoot: (labelId: number) =>
    [...queryKeys.all, 'labels', labelId, 'threads'] as const,
  labelThreads: (labelId: number, cursor?: string) =>
    [...queryKeys.labelThreadsRoot(labelId), cursor ?? null] as const,
  search: (q: string, scope: 'all' | 'mail' | 'notes' | 'clips', mailbox: string, labelId?: number) =>
    [...queryKeys.views(), 'search', scope, mailbox, labelId ?? 'all-labels', q] as const,
  labels: () => [...queryKeys.all, 'labels'] as const,
  screener: () => [...queryKeys.views(), 'screener'] as const,
  screenerAllowed: () => [...queryKeys.screener(), 'allowed'] as const,
  speakeasy: () => [...queryKeys.all, 'speakeasy'] as const,
  screenerDenied: () => [...queryKeys.screener(), 'denied'] as const,
  contacts: () => [...queryKeys.all, 'contacts'] as const,
  contact: (address: string) =>
    [...queryKeys.contacts(), address.trim().toLowerCase()] as const,
  threads: () => [...queryKeys.all, 'threads'] as const,
  thread: (threadId: string) => [...queryKeys.threads(), threadId] as const,
  drafts: () => [...queryKeys.all, 'drafts'] as const,
  draft: (draftId: string) => [...queryKeys.drafts(), draftId] as const,
  scheduledSends: () => [...queryKeys.all, 'scheduled-sends'] as const,
  workflows: () => [...queryKeys.all, 'workflows'] as const,
  providerSyncStatuses: () => [...queryKeys.all, 'provider-sync-statuses'] as const,
};
