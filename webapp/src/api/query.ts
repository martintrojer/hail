import {
  useMutation,
  useQuery,
  useQueryClient,
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
  type BubbleUpRequest,
  type BubbleUpResponse,
  type BubbleUpViewResponse,
  type CancelBubbleUpResponse,
  type ContactNote,
  type ContactResponse,
  type ComposeRequest,
  type ComposeResponse,
  type CreateAdminUserRequest,
  type DeniedSendersResponse,
  type DestroyThreadResponse,
  type DraftRequest,
  type DraftResponse,
  type LoginRequest,
  type MailViewResponse,
  type PileViewResponse,
  type PutContactNoteRequest,
  type RestoreThreadResponse,
  type ScreenerDecisionRequest,
  type ScreenerDecisionResponse,
  type ScreenerView,
  type UndoDenyResponse,
  type SearchParams,
  type SearchResponse,
  type SetupAdminRequest,
  type SetupState,
  type ThreadVerbResponse,
  type ThreadViewResponse,
  type TrashViewResponse,
  type UserEnvelope,
} from './client';
import { queryKeys } from './queryKeys';

const defaultBaseUrl =
  typeof window === 'undefined' ? 'http://localhost' : window.location.origin;

export const defaultApiClient = new HailApiClient({ baseUrl: defaultBaseUrl });

type QueryConfig<TData> = Omit<
  UseQueryOptions<TData, Error, TData, readonly unknown[]>,
  'queryKey' | 'queryFn'
>;

type MutationConfig<TVariables, TData> = Omit<
  UseMutationOptions<TData, Error, TVariables>,
  'mutationFn'
>;

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

export function useImboxView(
  client = defaultApiClient,
  options?: QueryConfig<MailViewResponse>,
) {
  return useQuery({
    queryKey: queryKeys.view('imbox'),
    queryFn: () => client.getImbox(),
    ...options,
  });
}

export function useFeedView(
  client = defaultApiClient,
  options?: QueryConfig<MailViewResponse>,
) {
  return useQuery({
    queryKey: queryKeys.view('feed'),
    queryFn: () => client.getFeed(),
    ...options,
  });
}

export function usePapertrailView(
  client = defaultApiClient,
  options?: QueryConfig<MailViewResponse>,
) {
  return useQuery({
    queryKey: queryKeys.view('papertrail'),
    queryFn: () => client.getPapertrail(),
    ...options,
  });
}

export function useDraftsView(
  client = defaultApiClient,
  options?: QueryConfig<MailViewResponse>,
) {
  return useQuery({
    queryKey: queryKeys.view('drafts'),
    queryFn: () => client.getDrafts(),
    ...options,
  });
}

export function useTrashView(
  client = defaultApiClient,
  options?: QueryConfig<TrashViewResponse>,
) {
  return useQuery({
    queryKey: queryKeys.view('trash'),
    queryFn: () => client.getTrash(),
    ...options,
  });
}

export function useSearch(
  params: SearchParams,
  client = defaultApiClient,
  options?: QueryConfig<SearchResponse>,
) {
  const normalizedQuery = params.q.trim();
  const scope = params.scope ?? 'all';

  return useQuery({
    queryKey: queryKeys.search(normalizedQuery, scope),
    queryFn: () => client.search({ q: normalizedQuery, scope }),
    ...options,
    enabled: normalizedQuery.length >= 2 && (options?.enabled ?? true),
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

export function useSetAsideView(
  client = defaultApiClient,
  options?: QueryConfig<PileViewResponse>,
) {
  return useQuery({
    queryKey: queryKeys.view('set-aside'),
    queryFn: () => client.getSetAside(),
    ...options,
  });
}

export function useReplyLaterView(
  client = defaultApiClient,
  options?: QueryConfig<PileViewResponse>,
) {
  return useQuery({
    queryKey: queryKeys.view('reply-later'),
    queryFn: () => client.getReplyLater(),
    ...options,
  });
}

export function useBubbleUpView(
  client = defaultApiClient,
  options?: QueryConfig<BubbleUpViewResponse>,
) {
  return useQuery({
    queryKey: queryKeys.view('bubble-up'),
    queryFn: () => client.getBubbleUps(),
    ...options,
  });
}

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
  options?: MutationConfig<ScreenerDecisionRequest, ScreenerDecisionResponse>,
) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (body) => client.decideScreener(body),
    ...options,
    onSuccess: (data, variables, onMutateResult, mutationContext) => {
      void queryClient.invalidateQueries({ queryKey: queryKeys.screener() });
      void queryClient.invalidateQueries({ queryKey: queryKeys.views() });
      options?.onSuccess?.(data, variables, onMutateResult, mutationContext);
    },
  });
}

export function useUndoDenyMutation(
  client = defaultApiClient,
  options?: MutationConfig<string, UndoDenyResponse>,
) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (address) => client.undoDeny(address),
    ...options,
    onSuccess: (data, variables, onMutateResult, mutationContext) => {
      void queryClient.invalidateQueries({ queryKey: queryKeys.screener() });
      void queryClient.invalidateQueries({ queryKey: queryKeys.views() });
      options?.onSuccess?.(data, variables, onMutateResult, mutationContext);
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

export function useSendComposeMutation(
  client = defaultApiClient,
  options?: MutationConfig<SendComposeMutationVariables, ComposeResponse>,
) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: ({ threadId, request }) =>
      threadId
        ? client.sendReply(threadId, {
            body_markdown: request.body_markdown,
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
