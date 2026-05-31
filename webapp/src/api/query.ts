import {
  useInfiniteQuery,
  useMutation,
  useQuery,
  useQueryClient,
  type InfiniteData,
  type QueryClient,
  type QueryKey,
  type UseMutationOptions,
  type UseQueryOptions,
} from '@tanstack/react-query';
import {
  HailApiClient,
  type AddAdminDomainRequest,
  type AdminDomainResponse,
  type AdminDomainsResponse,
  type AdminStatsResponse,
  type AdminUsersResponse,
  type AssignLabelNameRequest,
  type BatchAssignLabelRequest,
  type AttachmentsResponse,
  type BubbleUpRequest,
  type BubbleUpResponse,
  type CancelBubbleUpResponse,
  type ContactNote,
  type ContactResponse,
  type ComposeRequest,
  type ComposeResponse,
  type CreateAdminUserRequest,
  type CreateInviteRequest,
  type CreateLabelRequest,
  type CreatedInviteEnvelope,
  type DeniedSendersResponse,
  type DestroyThreadResponse,
  type DraftDetails,
  type DraftRequest,
  type DraftResponse,
  type ImboxSectionedResponse,
  type SectionedMailViewResponse,
  type LoginRequest,
  type PutContactNoteRequest,
  type RestoreThreadResponse,
  type RenameLabelRequest,
  type RotateSpeakeasyResponse,
  type ScheduledSend,
  type ScheduledSendsResponse,
  type ScreenerAllowedView,
  type ScreenerDecisionRequest,
  type ScreenerDecisionResponse,
  type ScreenerView,
  type SpamThreadResponse,
  type SpeakeasyResponse,
  type NotSpamThreadResponse,
  type UndoDenyRequest,
  type UndoDenyResponse,
  type UserPrefs,
  type UpdateUserPrefsRequest,
  type SearchParams,
  type SearchResponse,
  type SetupAdminRequest,
  type SetupState,
  type AcceptInviteRequest,
  type InviteAcceptResponse,
  type InvitePreview,
  type GmailConnectResponse,
  type LabelListResponse,
  type LabelItemResponse,
  type ProviderAccountResponse,
  type ProviderReimportResponse,
  type ProviderStopSyncResponse,
  type ProviderSyncStatusListResponse,
  type ProviderSyncTriggerResponse,
  type ProviderSyncStatus,
  type ThreadVerbResponse,
  type ThreadViewResponse,
  type UserEnvelope,
  type ViewCountsResponse,
  type CancelScheduledSendResponse,
  type WorkflowRuleListResponse,
  type WorkflowRulePayload,
  type WorkflowRuleResponse,
} from './client';
import { queryKeys } from './queryKeys';

const defaultBaseUrl =
  typeof window === 'undefined' ? 'http://localhost' : window.location.origin;

export const defaultApiClient = new HailApiClient({ baseUrl: defaultBaseUrl });

type QueryConfig<TData> = Omit<
  UseQueryOptions<TData, Error, TData, readonly unknown[]>,
  'queryKey' | 'queryFn'
>;

type MutationConfig<TVariables, TData, TContext = unknown> = Omit<
  UseMutationOptions<TData, Error, TVariables, TContext>,
  'mutationFn'
>;

type ViewKey = Parameters<typeof queryKeys.view>[0];
type ScreenerInfiniteData = InfiniteData<ScreenerView, string | undefined>;
type ScreenerCacheSnapshot = Array<[
  QueryKey,
  ScreenerInfiniteData | undefined,
]>;

function removeSenderFromScreenerInfiniteCache(
  queryClient: QueryClient,
  address: string,
): ScreenerCacheSnapshot {
  const normalizedAddress = address.trim().toLowerCase();
  const previous = queryClient.getQueriesData<ScreenerInfiniteData>({
    queryKey: queryKeys.screenerInfinite(),
  });

  queryClient.setQueriesData<ScreenerInfiniteData>(
    { queryKey: queryKeys.screenerInfinite() },
    (data) => {
      if (!data) {
        return data;
      }

      return {
        ...data,
        pages: data.pages.map((page) => ({
          ...page,
          senders: page.senders.filter(
            (sender) => sender.sender.trim().toLowerCase() !== normalizedAddress,
          ),
        })),
      };
    },
  );

  return previous;
}

function restoreScreenerInfiniteCache(
  queryClient: QueryClient,
  previous: ScreenerCacheSnapshot | undefined,
) {
  previous?.forEach(([queryKey, data]) => {
    queryClient.setQueryData(queryKey, data);
  });
}

function invalidateScreenerInfinite(queryClient: QueryClient) {
  void queryClient.invalidateQueries({
    queryKey: queryKeys.screenerInfinite(),
    refetchType: 'all',
  });
}


function createViewHook<TData>(
  key: ViewKey,
  fetcher: (client: HailApiClient) => Promise<TData>,
) {
  return (client = defaultApiClient, options?: QueryConfig<TData>) =>
    useQuery({
      queryKey: queryKeys.view(key),
      queryFn: () => fetcher(client),
      ...options,
    });
}

export function useMe(
  client = defaultApiClient,
  options?: QueryConfig<UserEnvelope>,
) {
  return useQuery({
    queryKey: queryKeys.me(),
    queryFn: () => client.me(),
    ...options,
  });
}

export function useSetupState(
  client = defaultApiClient,
  options?: QueryConfig<SetupState>,
) {
  return useQuery({
    queryKey: queryKeys.setupState(),
    queryFn: () => client.getSetupState(),
    ...options,
  });
}

export function useSetupAdminMutation(
  client = defaultApiClient,
  options?: MutationConfig<SetupAdminRequest, UserEnvelope>,
) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (body) => client.createSetupAdmin(body),
    ...options,
    onSuccess: (data, variables, onMutateResult, mutationContext) => {
      queryClient.setQueryData(queryKeys.me(), data);
      void queryClient.invalidateQueries({ queryKey: queryKeys.setup() });
      options?.onSuccess?.(data, variables, onMutateResult, mutationContext);
    },
  });
}

export function useAdminUsers(
  client = defaultApiClient,
  options?: QueryConfig<AdminUsersResponse>,
) {
  return useQuery({
    queryKey: queryKeys.adminUsers(),
    queryFn: () => client.listAdminUsers(),
    ...options,
  });
}

export function useAdminStats(
  client = defaultApiClient,
  options?: QueryConfig<AdminStatsResponse>,
) {
  return useQuery({
    queryKey: queryKeys.adminStats(),
    queryFn: () => client.getAdminStats(),
    ...options,
  });
}

export function useCreateAdminUserMutation(
  client = defaultApiClient,
  options?: MutationConfig<CreateAdminUserRequest, UserEnvelope>,
) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (body) => client.createAdminUser(body),
    ...options,
    onSuccess: (data, variables, onMutateResult, mutationContext) => {
      void queryClient.invalidateQueries({ queryKey: queryKeys.adminUsers() });
      options?.onSuccess?.(data, variables, onMutateResult, mutationContext);
    },
  });
}

export function useCreateInviteMutation(
  client = defaultApiClient,
  options?: MutationConfig<CreateInviteRequest, CreatedInviteEnvelope>,
) {
  return useMutation({
    mutationFn: (body) => client.createInvite(body),
    ...options,
  });
}

export function useInvite(
  token: string,
  client = defaultApiClient,
  options?: QueryConfig<InvitePreview>,
) {
  return useQuery({
    queryKey: queryKeys.invite(token),
    queryFn: () => client.getInvite(token),
    ...options,
  });
}

export function useAcceptInviteMutation(
  client = defaultApiClient,
  options?: MutationConfig<{ token: string; body: AcceptInviteRequest }, InviteAcceptResponse>,
) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: ({ token, body }) => client.acceptInvite(token, body),
    ...options,
    onSuccess: (data, variables, onMutateResult, mutationContext) => {
      queryClient.setQueryData(queryKeys.me(), data);
      options?.onSuccess?.(data, variables, onMutateResult, mutationContext);
    },
  });
}

export function useConnectGmailMutation(
  client = defaultApiClient,
  options?: MutationConfig<void, GmailConnectResponse>,
) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: () => client.connectGmail(),
    ...options,
    onSuccess: (data, variables, onMutateResult, mutationContext) => {
      void queryClient.invalidateQueries({ queryKey: queryKeys.providerSyncStatuses() });
      options?.onSuccess?.(data, variables, onMutateResult, mutationContext);
    },
  });
}

function applyProviderAccountResponseToSyncStatus(
  current: ProviderSyncStatusListResponse | undefined,
  updated: ProviderAccountResponse,
  fallbackId?: number,
): ProviderSyncStatusListResponse | undefined {
  if (!current) {
    return current;
  }

  return {
    accounts: current.accounts.map((account) =>
      account.id === updated.id || (fallbackId !== undefined && account.id === fallbackId)
        ? {
            ...account,
            display_email: updated.display_email,
            last_profile_history_id: updated.last_profile_history_id,
            provider_account_id: updated.provider_account_id,
            provider_email: updated.provider_email,
            provider_kind: updated.provider_kind,
            sync_status: updated.sync_status,
          }
        : account,
    ),
  };
}

export function useDisconnectProviderAccountMutation(
  client = defaultApiClient,
  options?: MutationConfig<number, ProviderAccountResponse>,
) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (id) => client.disconnectProviderAccount(id),
    ...options,
    onSuccess: (data, variables, onMutateResult, mutationContext) => {
      queryClient.setQueryData<ProviderSyncStatusListResponse>(
        queryKeys.providerSyncStatuses(),
        (current) => applyProviderAccountResponseToSyncStatus(current, data, variables),
      );
      void queryClient.invalidateQueries({ queryKey: queryKeys.providerSyncStatuses() });
      options?.onSuccess?.(data, variables, onMutateResult, mutationContext);
    },
  });
}

function isProviderSyncing(status: ProviderSyncStatus) {
  return status.sync_status === 'initial_sync' || Boolean(
    status.last_sync_attempted_at &&
    (!status.last_sync_succeeded_at ||
      new Date(status.last_sync_attempted_at) > new Date(status.last_sync_succeeded_at)),
  );
}

export function useProviderSyncStatuses(
  client = defaultApiClient,
  options?: QueryConfig<ProviderSyncStatusListResponse>,
) {
  return useQuery({
    queryKey: queryKeys.providerSyncStatuses(),
    queryFn: () => client.listProviderSyncStatuses(),
    refetchInterval: (query) => (
      query.state.data?.accounts.some(isProviderSyncing) ? 5000 : false
    ),
    ...options,
  });
}

export function useTriggerProviderSyncMutation(
  client = defaultApiClient,
  options?: MutationConfig<number, ProviderSyncTriggerResponse>,
) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (id) => client.triggerProviderSync(id),
    ...options,
    onSuccess: (data, variables, onMutateResult, mutationContext) => {
      queryClient.setQueryData<ProviderSyncStatusListResponse>(
        queryKeys.providerSyncStatuses(),
        (current) => {
          if (!current) {
            return current;
          }
          return {
            accounts: current.accounts.map((account) =>
              account.id === data.account.id ? data.account : account,
            ),
          };
        },
      );
      void queryClient.invalidateQueries({ queryKey: queryKeys.providerSyncStatuses() });
      options?.onSuccess?.(data, variables, onMutateResult, mutationContext);
    },
  });
}

export function useStopProviderSyncMutation(
  client = defaultApiClient,
  options?: MutationConfig<number, ProviderStopSyncResponse>,
) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (id) => client.stopProviderSync(id),
    ...options,
    onSuccess: (data, variables, onMutateResult, mutationContext) => {
      queryClient.setQueryData<ProviderSyncStatusListResponse>(
        queryKeys.providerSyncStatuses(),
        (current) => {
          if (!current) {
            return current;
          }
          return {
            accounts: current.accounts.map((account) =>
              account.id === data.account.id ? data.account : account,
            ),
          };
        },
      );
      void queryClient.invalidateQueries({ queryKey: queryKeys.providerSyncStatuses() });
      options?.onSuccess?.(data, variables, onMutateResult, mutationContext);
    },
  });
}

export function useReimportProviderAccountMutation(
  client = defaultApiClient,
  options?: MutationConfig<number, ProviderReimportResponse>,
) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (id) => client.reimportProviderAccount(id),
    ...options,
    onMutate: async (id) => {
      await queryClient.cancelQueries({ queryKey: queryKeys.providerSyncStatuses() });
      queryClient.setQueryData<ProviderSyncStatusListResponse>(
        queryKeys.providerSyncStatuses(),
        (current) => {
          if (!current) {
            return current;
          }
          return {
            accounts: current.accounts.map((account) =>
              account.id === id
                ? {
                    ...account,
                    sync_status: 'initial_sync',
                    last_profile_history_id: null,
                    last_error_class: null,
                    last_error_message: null,
                    last_error_event: null,
                    next_sync_after: null,
                    sync_backoff_secs: null,
                  }
                : account,
            ),
          };
        },
      );
      return undefined;
    },
    onSuccess: (data, variables, onMutateResult, mutationContext) => {
      queryClient.setQueryData<ProviderSyncStatusListResponse>(
        queryKeys.providerSyncStatuses(),
        (current) => {
          if (!current) {
            return current;
          }
          return {
            accounts: current.accounts.map((account) =>
              account.id === data.account.id ? data.account : account,
            ),
          };
        },
      );
      void queryClient.invalidateQueries({ queryKey: queryKeys.providerSyncStatuses() });
      options?.onSuccess?.(data, variables, onMutateResult, mutationContext);
    },
    onError: (error, variables, onMutateResult, mutationContext) => {
      void queryClient.invalidateQueries({ queryKey: queryKeys.providerSyncStatuses() });
      options?.onError?.(error, variables, onMutateResult, mutationContext);
    },
  });
}

export function useDeleteAdminUserMutation(
  client = defaultApiClient,
  options?: MutationConfig<number, void>,
) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (userId) => client.deleteAdminUser(userId),
    ...options,
    onSuccess: (data, variables, onMutateResult, mutationContext) => {
      void queryClient.invalidateQueries({ queryKey: queryKeys.adminUsers() });
      options?.onSuccess?.(data, variables, onMutateResult, mutationContext);
    },
  });
}

export interface ResetAdminUserPasswordVariables {
  userId: number;
  password: string;
}

export function useResetAdminUserPasswordMutation(
  client = defaultApiClient,
  options?: MutationConfig<ResetAdminUserPasswordVariables, UserEnvelope>,
) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: ({ userId, password }) =>
      client.resetAdminUserPassword(userId, { password }),
    ...options,
    onSuccess: (data, variables, onMutateResult, mutationContext) => {
      void queryClient.invalidateQueries({ queryKey: queryKeys.adminUsers() });
      options?.onSuccess?.(data, variables, onMutateResult, mutationContext);
    },
  });
}

export function useAdminDomains(
  client = defaultApiClient,
  options?: QueryConfig<AdminDomainsResponse>,
) {
  return useQuery({
    queryKey: queryKeys.adminDomains(),
    queryFn: () => client.listAdminDomains(),
    ...options,
  });
}

export function useAddAdminDomainMutation(
  client = defaultApiClient,
  options?: MutationConfig<AddAdminDomainRequest, AdminDomainResponse>,
) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (body) => client.addAdminDomain(body),
    ...options,
    onSuccess: (data, variables, onMutateResult, mutationContext) => {
      void queryClient.invalidateQueries({ queryKey: queryKeys.adminDomains() });
      options?.onSuccess?.(data, variables, onMutateResult, mutationContext);
    },
  });
}

export function useDeleteAdminDomainMutation(
  client = defaultApiClient,
  options?: MutationConfig<string, void>,
) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (domain) => client.deleteAdminDomain(domain),
    ...options,
    onSuccess: (data, variables, onMutateResult, mutationContext) => {
      void queryClient.invalidateQueries({ queryKey: queryKeys.adminDomains() });
      options?.onSuccess?.(data, variables, onMutateResult, mutationContext);
    },
  });
}

export function useScreenerView(
  client = defaultApiClient,
  options?: QueryConfig<ScreenerView>,
) {
  return useQuery({
    queryKey: queryKeys.screener(),
    queryFn: () => client.getScreenerView(),
    ...options,
  });
}

export function useScreenerInfiniteView(client = defaultApiClient) {
  return useInfiniteQuery({
    queryKey: queryKeys.screenerInfinite(),
    queryFn: ({ pageParam }) =>
      client.getScreenerView(
        typeof pageParam === 'string' ? { cursor: pageParam } : {},
      ),
    initialPageParam: undefined as string | undefined,
    getNextPageParam: (lastPage) => lastPage.next_cursor ?? undefined,
  });
}

export function useViewCounts(
  client = defaultApiClient,
  options?: QueryConfig<ViewCountsResponse>,
) {
  return useQuery({
    queryKey: queryKeys.viewCounts(),
    queryFn: () => client.getViewCounts(),
    ...options,
  });
}

export function useUserPrefs(
  client = defaultApiClient,
  options?: QueryConfig<UserPrefs>,
) {
  return useQuery({
    queryKey: queryKeys.userPrefs(),
    queryFn: () => client.getUserPrefs(),
    ...options,
  });
}

export function useUpdateUserPrefsMutation(
  client = defaultApiClient,
  options?: MutationConfig<UpdateUserPrefsRequest, UserPrefs>,
) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (body) => client.updateUserPrefs(body),
    ...options,
    onSuccess: (data, variables, onMutateResult, mutationContext) => {
      queryClient.setQueryData(queryKeys.userPrefs(), data);
      void queryClient.invalidateQueries({ queryKey: queryKeys.view('feed') });
      options?.onSuccess?.(data, variables, onMutateResult, mutationContext);
    },
  });
}

export function useScreenerAllowedView(
  client = defaultApiClient,
  options?: QueryConfig<ScreenerAllowedView>,
) {
  return useQuery({
    queryKey: queryKeys.screenerAllowed(),
    queryFn: () => client.getScreenerAllowedView(),
    ...options,
  });
}

export function useSpeakeasy(
  client = defaultApiClient,
  options?: QueryConfig<SpeakeasyResponse>,
) {
  return useQuery({
    queryKey: queryKeys.speakeasy(),
    queryFn: () => client.getSpeakeasy(),
    ...options,
  });
}

export function useRotateSpeakeasyMutation(
  client = defaultApiClient,
  options?: MutationConfig<void, RotateSpeakeasyResponse>,
) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: () =>
      client.rotateSpeakeasy({ acknowledge_bypass_secret: true }),
    ...options,
    onSuccess: (data, variables, onMutateResult, mutationContext) => {
      queryClient.setQueryData(queryKeys.speakeasy(), data);
      void queryClient.invalidateQueries({ queryKey: queryKeys.speakeasy() });
      options?.onSuccess?.(data, variables, onMutateResult, mutationContext);
    },
  });
}

export function useDeniedSenders(
  client = defaultApiClient,
  options?: QueryConfig<DeniedSendersResponse>,
) {
  return useQuery({
    queryKey: queryKeys.screenerDenied(),
    queryFn: () => client.getDeniedSenders(),
    ...options,
  });
}

export const useImboxView = createViewHook('imbox', (client) => client.getImbox());

export function useImboxSectioned(
  client = defaultApiClient,
  options?: QueryConfig<ImboxSectionedResponse>,
) {
  return useQuery({
    queryKey: queryKeys.imboxSectioned(),
    queryFn: () => client.getImboxSectioned(),
    ...options,
  });
}

export function usePapertrailSectioned(
  client = defaultApiClient,
  options?: QueryConfig<SectionedMailViewResponse>,
) {
  return useQuery({
    queryKey: queryKeys.viewSectioned('papertrail'),
    queryFn: () => client.getPapertrailSectioned(),
    ...options,
  });
}

export const useFeedView = createViewHook('feed', (client) => client.getFeed());

export const usePapertrailView = createViewHook('papertrail', (client) =>
  client.getPapertrail(),
);

export const useDraftsView = createViewHook('drafts', (client) => client.getDrafts());

export const useArchiveView = createViewHook('archive', (client) =>
  client.getArchiveView(),
);

export const useTrashView = createViewHook('trash', (client) => client.getTrash());

export const useSpamView = createViewHook('spam', (client) =>
  client.getSpamView(),
);

export function useAttachments(
  client = defaultApiClient,
  options?: QueryConfig<AttachmentsResponse>,
) {
  return useQuery({
    queryKey: queryKeys.attachments(),
    queryFn: () => client.listAttachments(),
    ...options,
  });
}

export function useScheduledSends(
  client = defaultApiClient,
  options?: QueryConfig<ScheduledSendsResponse>,
) {
  return useQuery({
    queryKey: queryKeys.scheduledSends(),
    queryFn: () => client.listScheduledSends(),
    ...options,
  });
}

export function useWorkflows(
  client = defaultApiClient,
  options?: QueryConfig<WorkflowRuleListResponse>,
) {
  return useQuery({
    queryKey: queryKeys.workflows(),
    queryFn: () => client.listWorkflows(),
    ...options,
  });
}

export function useLabelThreads(
  labelId: number,
  client = defaultApiClient,
  options?: { enabled?: boolean },
) {
  return useInfiniteQuery({
    queryKey: queryKeys.labelThreadsRoot(labelId),
    queryFn: ({ pageParam }) =>
      client.getLabelThreads(
        labelId,
        typeof pageParam === 'string' ? { cursor: pageParam } : {},
      ),
    initialPageParam: undefined as string | undefined,
    getNextPageParam: (lastPage) => lastPage.next_cursor ?? undefined,
    ...options,
    enabled: Number.isSafeInteger(labelId) && labelId > 0 && (options?.enabled ?? true),
  });
}

export function useSearch(
  params: SearchParams,
  client = defaultApiClient,
  options?: QueryConfig<SearchResponse>,
) {
  const normalizedQuery = params.q.trim();
  const scope = params.scope ?? 'all';
  const mailbox = params.mailbox ?? 'all';
  const labelId = params.label_id;

  return useQuery({
    queryKey: queryKeys.search(normalizedQuery, scope, mailbox, labelId),
    queryFn: () => client.search({ q: normalizedQuery, scope, mailbox, label_id: labelId }),
    ...options,
    enabled: normalizedQuery.length >= 2 && (options?.enabled ?? true),
  });
}

export function useLabels(
  client = defaultApiClient,
  options?: QueryConfig<LabelListResponse>,
) {
  return useQuery({
    queryKey: queryKeys.labels(),
    queryFn: () => client.listLabels(),
    ...options,
  });
}

function upsertLabelInList(
  current: LabelListResponse | undefined,
  updated: LabelItemResponse,
): LabelListResponse | undefined {
  if (!current) {
    return current;
  }

  const exists = current.labels.some((label) => label.id === updated.label.id);
  return {
    labels: exists
      ? current.labels.map((label) => (label.id === updated.label.id ? updated.label : label))
      : [...current.labels, updated.label],
  };
}

function removeLabelFromList(
  current: LabelListResponse | undefined,
  labelId: number,
): LabelListResponse | undefined {
  if (!current) {
    return current;
  }

  return {
    labels: current.labels.filter((label) => label.id !== labelId),
  };
}

function refreshLabelCaches(queryClient: QueryClient, labelId?: number) {
  void queryClient.invalidateQueries({ queryKey: queryKeys.labels() });
  if (labelId !== undefined) {
    void queryClient.invalidateQueries({ queryKey: queryKeys.labelThreadsRoot(labelId) });
  }
  void queryClient.invalidateQueries({ queryKey: queryKeys.views() });
}

export function useCreateLabelMutation(
  client = defaultApiClient,
  options?: MutationConfig<CreateLabelRequest, LabelItemResponse>,
) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (body) => client.createLabel(body),
    ...options,
    onSuccess: (data, variables, onMutateResult, mutationContext) => {
      queryClient.setQueryData<LabelListResponse | undefined>(queryKeys.labels(), (current) =>
        upsertLabelInList(current, data),
      );
      refreshLabelCaches(queryClient, data.label.id);
      options?.onSuccess?.(data, variables, onMutateResult, mutationContext);
    },
  });
}

function addThreadLabel(
  current: ThreadViewResponse | undefined,
  label: LabelItemResponse['label'],
): ThreadViewResponse | undefined {
  if (!current) {
    return current;
  }
  if (current.labels.some((existing) => existing.id === label.id)) {
    return current;
  }
  return {
    ...current,
    labels: [...current.labels, label],
  };
}

function removeThreadLabel(
  current: ThreadViewResponse | undefined,
  labelId: number,
): ThreadViewResponse | undefined {
  if (!current) {
    return current;
  }
  return {
    ...current,
    labels: current.labels.filter((label) => label.id !== labelId),
  };
}

export function useAssignLabelToThreadMutation(
  client = defaultApiClient,
  options?: MutationConfig<{ threadId: string; labelId: number }, LabelItemResponse>,
) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: ({ threadId, labelId }) => client.assignLabelToThread(threadId, labelId),
    ...options,
    onSuccess: (data, variables, onMutateResult, mutationContext) => {
      queryClient.setQueryData<ThreadViewResponse | undefined>(
        queryKeys.thread(variables.threadId),
        (current) => addThreadLabel(current, data.label),
      );
      void queryClient.invalidateQueries({ queryKey: queryKeys.thread(variables.threadId) });
      void queryClient.invalidateQueries({ queryKey: queryKeys.labels() });
      void queryClient.invalidateQueries({ queryKey: queryKeys.views() });
      void queryClient.invalidateQueries({ queryKey: queryKeys.labelThreads(data.label.id) });
      options?.onSuccess?.(data, variables, onMutateResult, mutationContext);
    },
  });
}

export function useAssignLabelNameToThreadMutation(
  client = defaultApiClient,
  options?: MutationConfig<{ threadId: string; request: AssignLabelNameRequest }, LabelItemResponse>,
) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: ({ threadId, request }) => client.assignLabelNameToThread(threadId, request),
    ...options,
    onSuccess: (data, variables, onMutateResult, mutationContext) => {
      queryClient.setQueryData<ThreadViewResponse | undefined>(
        queryKeys.thread(variables.threadId),
        (current) => addThreadLabel(current, data.label),
      );
      void queryClient.invalidateQueries({ queryKey: queryKeys.thread(variables.threadId) });
      void queryClient.invalidateQueries({ queryKey: queryKeys.labels() });
      void queryClient.invalidateQueries({ queryKey: queryKeys.views() });
      void queryClient.invalidateQueries({ queryKey: queryKeys.labelThreads(data.label.id) });
      options?.onSuccess?.(data, variables, onMutateResult, mutationContext);
    },
  });
}

export function useAssignLabelToThreadsMutation(
  client = defaultApiClient,
  options?: MutationConfig<BatchAssignLabelRequest, LabelItemResponse>,
) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (request) => client.assignLabelToThreads(request),
    ...options,
    onSuccess: (data, variables, onMutateResult, mutationContext) => {
      queryClient.setQueryData<LabelListResponse | undefined>(queryKeys.labels(), (current) =>
        upsertLabelInList(current, data),
      );
      for (const threadId of variables.thread_ids) {
        queryClient.setQueryData<ThreadViewResponse | undefined>(
          queryKeys.thread(threadId),
          (current) => addThreadLabel(current, data.label),
        );
        void queryClient.invalidateQueries({ queryKey: queryKeys.thread(threadId) });
      }
      void queryClient.invalidateQueries({ queryKey: queryKeys.labels() });
      void queryClient.invalidateQueries({ queryKey: queryKeys.views() });
      void queryClient.invalidateQueries({ queryKey: queryKeys.labelThreadsRoot(data.label.id) });
      options?.onSuccess?.(data, variables, onMutateResult, mutationContext);
    },
  });
}

export function useRemoveLabelFromThreadMutation(
  client = defaultApiClient,
  options?: MutationConfig<{ threadId: string; labelId: number }, void>,
) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: ({ threadId, labelId }) => client.removeLabelFromThread(threadId, labelId),
    ...options,
    onSuccess: (data, variables, onMutateResult, mutationContext) => {
      queryClient.setQueryData<ThreadViewResponse | undefined>(
        queryKeys.thread(variables.threadId),
        (current) => removeThreadLabel(current, variables.labelId),
      );
      void queryClient.invalidateQueries({ queryKey: queryKeys.thread(variables.threadId) });
      void queryClient.invalidateQueries({ queryKey: queryKeys.labels() });
      void queryClient.invalidateQueries({ queryKey: queryKeys.views() });
      void queryClient.invalidateQueries({ queryKey: queryKeys.labelThreads(variables.labelId) });
      options?.onSuccess?.(data, variables, onMutateResult, mutationContext);
    },
  });
}

export interface RenameLabelMutationVariables {
  id: number;
  request: RenameLabelRequest;
}

export function useRenameLabelMutation(
  client = defaultApiClient,
  options?: MutationConfig<RenameLabelMutationVariables, LabelItemResponse>,
) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: ({ id, request }) => client.renameLabel(id, request),
    ...options,
    onSuccess: (data, variables, onMutateResult, mutationContext) => {
      queryClient.setQueryData<LabelListResponse | undefined>(queryKeys.labels(), (current) =>
        upsertLabelInList(current, data),
      );
      refreshLabelCaches(queryClient, variables.id);
      options?.onSuccess?.(data, variables, onMutateResult, mutationContext);
    },
  });
}

export function useDeleteLabelMutation(
  client = defaultApiClient,
  options?: MutationConfig<number, void>,
) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (id) => client.deleteLabel(id),
    ...options,
    onSuccess: (data, variables, onMutateResult, mutationContext) => {
      queryClient.setQueryData<LabelListResponse | undefined>(queryKeys.labels(), (current) =>
        removeLabelFromList(current, variables),
      );
      void queryClient.removeQueries({ queryKey: queryKeys.labelThreadsRoot(variables) });
      refreshLabelCaches(queryClient, variables);
      options?.onSuccess?.(data, variables, onMutateResult, mutationContext);
    },
  });
}

export function useThread(
  threadId: string,
  client = defaultApiClient,
  options?: QueryConfig<ThreadViewResponse>,
) {
  return useQuery({
    queryKey: queryKeys.thread(threadId),
    queryFn: () => client.getThread(threadId),
    ...options,
    enabled: threadId.trim().length > 0 && (options?.enabled ?? true),
  });
}

export const useSetAsideView = createViewHook('set-aside', (client) =>
  client.getSetAside(),
);

export const useReplyLaterView = createViewHook('reply-later', (client) =>
  client.getReplyLater(),
);

export const useBubbleUpView = createViewHook('bubble-up', (client) =>
  client.getBubbleUps(),
);

export function useContact(
  address: string,
  client = defaultApiClient,
  options?: QueryConfig<ContactResponse>,
) {
  return useQuery({
    queryKey: queryKeys.contact(address),
    queryFn: () => client.getContact(address),
    ...options,
    enabled: address.trim().length > 0 && (options?.enabled ?? true),
  });
}

export function useLoginMutation(
  client = defaultApiClient,
  options?: MutationConfig<LoginRequest, UserEnvelope>,
) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (body) => client.login(body),
    ...options,
    onSuccess: (data, variables, onMutateResult, mutationContext) => {
      queryClient.setQueryData(queryKeys.me(), data);
      void queryClient.invalidateQueries({ queryKey: queryKeys.setup() });
      options?.onSuccess?.(data, variables, onMutateResult, mutationContext);
    },
  });
}

export function useLogoutMutation(
  client = defaultApiClient,
  options?: MutationConfig<void, void>,
) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: () => client.logout(),
    ...options,
    onSuccess: (data, variables, onMutateResult, mutationContext) => {
      queryClient.removeQueries({ queryKey: queryKeys.auth() });
      void queryClient.invalidateQueries({ queryKey: queryKeys.all });
      options?.onSuccess?.(data, variables, onMutateResult, mutationContext);
    },
  });
}

export function useScreenerDecisionMutation(
  client = defaultApiClient,
  options?: MutationConfig<
    ScreenerDecisionRequest,
    ScreenerDecisionResponse,
    ScreenerCacheSnapshot
  >,
) {
  const queryClient = useQueryClient();

  return useMutation<
    ScreenerDecisionResponse,
    Error,
    ScreenerDecisionRequest,
    ScreenerCacheSnapshot
  >({
    mutationFn: (body) => client.decideScreener(body),
    ...options,
    onMutate: async (variables) => {
      void queryClient.cancelQueries({ queryKey: queryKeys.screenerInfinite() });
      const previousScreenerInfinite = removeSenderFromScreenerInfiniteCache(
        queryClient,
        variables.sender,
      );
      return previousScreenerInfinite;
    },
    onError: (error, variables, previousScreenerInfinite, mutationContext) => {
      restoreScreenerInfiniteCache(
        queryClient,
        previousScreenerInfinite,
      );
      options?.onError?.(
        error,
        variables,
        previousScreenerInfinite,
        mutationContext,
      );
    },
    onSuccess: (data, variables, onMutateResult, mutationContext) => {
      void queryClient.invalidateQueries({ queryKey: queryKeys.views() });
      options?.onSuccess?.(
        data,
        variables,
        onMutateResult,
        mutationContext,
      );
    },
    onSettled: (data, error, variables, onMutateResult, mutationContext) => {
      invalidateScreenerInfinite(queryClient);
      options?.onSettled?.(
        data,
        error,
        variables,
        onMutateResult,
        mutationContext,
      );
    },
  });
}

export interface UndoDenyMutationVariables {
  address: string;
  classify_as?: UndoDenyRequest['classify_as'];
}

export function useUndoDenyMutation(
  client = defaultApiClient,
  options?: MutationConfig<
    UndoDenyMutationVariables,
    UndoDenyResponse,
    ScreenerCacheSnapshot
  >,
) {
  const queryClient = useQueryClient();

  return useMutation<
    UndoDenyResponse,
    Error,
    UndoDenyMutationVariables,
    ScreenerCacheSnapshot
  >({
    mutationFn: ({ address, classify_as }) =>
      client.undoDeny(address, { classify_as: classify_as ?? null }),
    ...options,
    onMutate: async (variables) => {
      void queryClient.cancelQueries({ queryKey: queryKeys.screenerInfinite() });
      const previousScreenerInfinite = removeSenderFromScreenerInfiniteCache(
        queryClient,
        variables.address,
      );
      return previousScreenerInfinite;
    },
    onError: (error, variables, previousScreenerInfinite, mutationContext) => {
      restoreScreenerInfiniteCache(
        queryClient,
        previousScreenerInfinite,
      );
      options?.onError?.(
        error,
        variables,
        previousScreenerInfinite,
        mutationContext,
      );
    },
    onSuccess: (data, variables, onMutateResult, mutationContext) => {
      void queryClient.invalidateQueries({ queryKey: queryKeys.views() });
      options?.onSuccess?.(
        data,
        variables,
        onMutateResult,
        mutationContext,
      );
    },
    onSettled: (data, error, variables, onMutateResult, mutationContext) => {
      invalidateScreenerInfinite(queryClient);
      options?.onSettled?.(
        data,
        error,
        variables,
        onMutateResult,
        mutationContext,
      );
    },
  });
}

export interface ContactNoteMutationVariables {
  address: string;
  note: PutContactNoteRequest | null;
}

export function useContactNoteMutation(
  client = defaultApiClient,
  options?: MutationConfig<ContactNoteMutationVariables, ContactNote | void>,
) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: ({ address, note }) =>
      note === null
        ? client.deleteContactNote(address)
        : client.putContactNote(address, note),
    ...options,
    onSuccess: (data, variables, onMutateResult, mutationContext) => {
      void queryClient.invalidateQueries({
        queryKey: queryKeys.contact(variables.address),
      });
      options?.onSuccess?.(data, variables, onMutateResult, mutationContext);
    },
  });
}

export interface ThreadVerbMutationVariables {
  threadId: string;
}

export interface ClassifyThreadMutationVariables extends ThreadVerbMutationVariables {
  to: 'imbox' | 'feed' | 'papertrail';
}

export function useClassifyThreadMutation(
  client = defaultApiClient,
  options?: MutationConfig<ClassifyThreadMutationVariables, ThreadVerbResponse>,
) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: ({ threadId, to }) => client.classifyThread(threadId, to),
    ...options,
    onSuccess: (data, variables, onMutateResult, mutationContext) => {
      void queryClient.invalidateQueries({
        queryKey: queryKeys.thread(variables.threadId),
      });
      void queryClient.invalidateQueries({ queryKey: queryKeys.views() });
      options?.onSuccess?.(data, variables, onMutateResult, mutationContext);
    },
  });
}

export function useArchiveThreadMutation(
  client = defaultApiClient,
  options?: MutationConfig<ThreadVerbMutationVariables, ThreadVerbResponse>,
) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: ({ threadId }) => client.archiveThread(threadId),
    ...options,
    onSuccess: (data, variables, onMutateResult, mutationContext) => {
      void queryClient.invalidateQueries({
        queryKey: queryKeys.thread(variables.threadId),
      });
      void queryClient.invalidateQueries({ queryKey: queryKeys.views() });
      options?.onSuccess?.(data, variables, onMutateResult, mutationContext);
    },
  });
}

export function useTrashThreadMutation(
  client = defaultApiClient,
  options?: MutationConfig<ThreadVerbMutationVariables, ThreadVerbResponse>,
) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: ({ threadId }) => client.trashThread(threadId),
    ...options,
    onSuccess: (data, variables, onMutateResult, mutationContext) => {
      void queryClient.invalidateQueries({
        queryKey: queryKeys.thread(variables.threadId),
      });
      void queryClient.invalidateQueries({ queryKey: queryKeys.views() });
      options?.onSuccess?.(data, variables, onMutateResult, mutationContext);
    },
  });
}

export function useSpamThreadMutation(
  client = defaultApiClient,
  options?: MutationConfig<ThreadVerbMutationVariables, SpamThreadResponse>,
) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: ({ threadId }) => client.spamThread(threadId),
    ...options,
    onSuccess: (data, variables, onMutateResult, mutationContext) => {
      void queryClient.invalidateQueries({
        queryKey: queryKeys.thread(variables.threadId),
      });
      void queryClient.invalidateQueries({ queryKey: queryKeys.views() });
      options?.onSuccess?.(data, variables, onMutateResult, mutationContext);
    },
  });
}

export function useNotSpamThreadMutation(
  client = defaultApiClient,
  options?: MutationConfig<ThreadVerbMutationVariables, NotSpamThreadResponse>,
) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: ({ threadId }) => client.notSpamThread(threadId),
    ...options,
    onSuccess: (data, variables, onMutateResult, mutationContext) => {
      void queryClient.invalidateQueries({
        queryKey: queryKeys.thread(variables.threadId),
      });
      void queryClient.invalidateQueries({ queryKey: queryKeys.views() });
      options?.onSuccess?.(data, variables, onMutateResult, mutationContext);
    },
  });
}

export function useRestoreThreadMutation(
  client = defaultApiClient,
  options?: MutationConfig<ThreadVerbMutationVariables, RestoreThreadResponse>,
) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: ({ threadId }) => client.restoreThread(threadId),
    ...options,
    onSuccess: (data, variables, onMutateResult, mutationContext) => {
      void queryClient.invalidateQueries({
        queryKey: queryKeys.thread(variables.threadId),
      });
      void queryClient.invalidateQueries({ queryKey: queryKeys.views() });
      options?.onSuccess?.(data, variables, onMutateResult, mutationContext);
    },
  });
}

export function useDestroyThreadMutation(
  client = defaultApiClient,
  options?: MutationConfig<ThreadVerbMutationVariables, DestroyThreadResponse>,
) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: ({ threadId }) => client.destroyThread(threadId),
    ...options,
    onSuccess: (data, variables, onMutateResult, mutationContext) => {
      void queryClient.removeQueries({ queryKey: queryKeys.thread(variables.threadId) });
      void queryClient.invalidateQueries({ queryKey: queryKeys.view('trash') });
      void queryClient.invalidateQueries({ queryKey: queryKeys.views() });
      options?.onSuccess?.(data, variables, onMutateResult, mutationContext);
    },
  });
}

export function useSetAsideThreadMutation(
  client = defaultApiClient,
  options?: MutationConfig<ThreadVerbMutationVariables, ThreadVerbResponse>,
) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: ({ threadId }) => client.setAsideThread(threadId),
    ...options,
    onSuccess: (data, variables, onMutateResult, mutationContext) => {
      void queryClient.invalidateQueries({
        queryKey: queryKeys.thread(variables.threadId),
      });
      void queryClient.invalidateQueries({ queryKey: queryKeys.views() });
      options?.onSuccess?.(data, variables, onMutateResult, mutationContext);
    },
  });
}

export function useReplyLaterThreadMutation(
  client = defaultApiClient,
  options?: MutationConfig<ThreadVerbMutationVariables, ThreadVerbResponse>,
) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: ({ threadId }) => client.replyLaterThread(threadId),
    ...options,
    onSuccess: (data, variables, onMutateResult, mutationContext) => {
      void queryClient.invalidateQueries({
        queryKey: queryKeys.thread(variables.threadId),
      });
      void queryClient.invalidateQueries({ queryKey: queryKeys.views() });
      options?.onSuccess?.(data, variables, onMutateResult, mutationContext);
    },
  });
}

export interface BubbleUpMutationVariables {
  threadId: string;
  request: BubbleUpRequest;
}

export function useBubbleUpMutation(
  client = defaultApiClient,
  options?: MutationConfig<BubbleUpMutationVariables, BubbleUpResponse>,
) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: ({ threadId, request }) => client.bubbleUpThread(threadId, request),
    ...options,
    onSuccess: (data, variables, onMutateResult, mutationContext) => {
      void queryClient.invalidateQueries({
        queryKey: queryKeys.thread(variables.threadId),
      });
      void queryClient.invalidateQueries({ queryKey: queryKeys.views() });
      options?.onSuccess?.(data, variables, onMutateResult, mutationContext);
    },
  });
}

export interface CancelBubbleUpMutationVariables {
  threadId: string;
}

export function useCancelBubbleUpMutation(
  client = defaultApiClient,
  options?: MutationConfig<CancelBubbleUpMutationVariables, CancelBubbleUpResponse>,
) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: ({ threadId }) => client.cancelBubbleUp(threadId),
    ...options,
    onSuccess: (data, variables, onMutateResult, mutationContext) => {
      void queryClient.invalidateQueries({ queryKey: queryKeys.view('bubble-up') });
      void queryClient.invalidateQueries({
        queryKey: queryKeys.thread(variables.threadId),
      });
      options?.onSuccess?.(data, variables, onMutateResult, mutationContext);
    },
  });
}

export interface SendComposeMutationVariables {
  threadId?: string;
  request: ComposeRequest;
}

export function useCancelScheduledSendMutation(
  client = defaultApiClient,
  options?: MutationConfig<number, CancelScheduledSendResponse>,
) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (scheduledSendId) => client.cancelScheduledSend(scheduledSendId),
    ...options,
    onSuccess: (data, variables, onMutateResult, mutationContext) => {
      queryClient.setQueryData<ScheduledSend[] | undefined>(
        queryKeys.scheduledSends(),
        (current) => current?.map((item) => (item.id === data.id ? data : item)),
      );
      void queryClient.invalidateQueries({ queryKey: queryKeys.scheduledSends() });
      options?.onSuccess?.(data, variables, onMutateResult, mutationContext);
    },
  });
}

export function useCreateWorkflowMutation(
  client = defaultApiClient,
  options?: MutationConfig<WorkflowRulePayload, WorkflowRuleResponse>,
) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (body) => client.createWorkflow(body),
    ...options,
    onSuccess: (data, variables, onMutateResult, mutationContext) => {
      void queryClient.invalidateQueries({ queryKey: queryKeys.workflows() });
      options?.onSuccess?.(data, variables, onMutateResult, mutationContext);
    },
  });
}

export interface UpdateWorkflowMutationVariables {
  id: number;
  request: WorkflowRulePayload;
}

export function useUpdateWorkflowMutation(
  client = defaultApiClient,
  options?: MutationConfig<UpdateWorkflowMutationVariables, WorkflowRuleResponse>,
) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: ({ id, request }) => client.updateWorkflow(id, request),
    ...options,
    onSuccess: (data, variables, onMutateResult, mutationContext) => {
      void queryClient.invalidateQueries({ queryKey: queryKeys.workflows() });
      options?.onSuccess?.(data, variables, onMutateResult, mutationContext);
    },
  });
}

export function useDeleteWorkflowMutation(
  client = defaultApiClient,
  options?: MutationConfig<number, void>,
) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (id) => client.deleteWorkflow(id),
    ...options,
    onSuccess: (data, variables, onMutateResult, mutationContext) => {
      queryClient.setQueryData<WorkflowRuleListResponse | undefined>(
        queryKeys.workflows(),
        (current) => current
          ? { rules: current.rules.filter((rule) => rule.id !== variables) }
          : current,
      );
      void queryClient.invalidateQueries({ queryKey: queryKeys.workflows() });
      options?.onSuccess?.(data, variables, onMutateResult, mutationContext);
    },
  });
}

export function useSendComposeMutation(
  client = defaultApiClient,
  options?: MutationConfig<SendComposeMutationVariables, ComposeResponse>,
) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: ({ threadId, request }) =>
      threadId
        ? client.sendReply(threadId, {
            body_html: request.body_html,
            attachments: request.attachments,
            send_at: request.send_at,
          })
        : client.sendCompose(request),
    ...options,
    onSuccess: (data, variables, onMutateResult, mutationContext) => {
      if (variables.threadId) {
        void queryClient.invalidateQueries({
          queryKey: queryKeys.thread(variables.threadId),
        });
      }
      void queryClient.invalidateQueries({ queryKey: queryKeys.views() });
      if (data.status === 'pending') {
        void queryClient.invalidateQueries({ queryKey: queryKeys.scheduledSends() });
      }
      options?.onSuccess?.(data, variables, onMutateResult, mutationContext);
    },
  });
}

export function useCreateDraftMutation(
  client = defaultApiClient,
  options?: MutationConfig<DraftRequest, DraftResponse>,
) {
  return useMutation({
    mutationFn: (request) => client.createDraft(request),
    ...options,
  });
}

export function useDraft(
  draftId: string | undefined,
  client = defaultApiClient,
  options?: QueryConfig<DraftDetails>,
) {
  return useQuery({
    queryKey: queryKeys.draft(draftId ?? ''),
    queryFn: () => client.getDraft(draftId ?? ''),
    ...options,
    enabled: Boolean(draftId) && (options?.enabled ?? true),
  });
}

export interface UpdateDraftMutationVariables {
  draftId: string;
  request: DraftRequest;
}

export function useUpdateDraftMutation(
  client = defaultApiClient,
  options?: MutationConfig<UpdateDraftMutationVariables, DraftResponse>,
) {
  return useMutation({
    mutationFn: ({ draftId, request }) => client.updateDraft(draftId, request),
    ...options,
  });
}

export function useDeleteDraftMutation(
  client = defaultApiClient,
  options?: MutationConfig<string, void>,
) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (draftId) => client.deleteDraft(draftId),
    ...options,
    onSuccess: (data, variables, onMutateResult, mutationContext) => {
      void queryClient.invalidateQueries({ queryKey: queryKeys.view('drafts') });
      options?.onSuccess?.(data, variables, onMutateResult, mutationContext);
    },
  });
}
