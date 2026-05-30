import type { components, paths } from './types';

type ResponseBody<Response> = Response extends {
  content: { 'application/json': infer Body };
}
  ? Body
  : undefined;

type HealthzSuccess = ResponseBody<
  paths['/healthz']['get']['responses']['204']
>;
type ReadyzSuccess = ResponseBody<paths['/readyz']['get']['responses']['200']>;
type ReadyzUnavailable = ResponseBody<
  paths['/readyz']['get']['responses']['503']
>;

type ThreadGetSuccess = ResponseBody<
  paths['/api/threads/{thread_id}']['get']['responses']['200']
>;
type DraftGetSuccess = ResponseBody<
  paths['/api/drafts/{draft_id}']['get']['responses']['200']
>;
type BlobUploadSuccess = ResponseBody<
  paths['/api/blobs']['post']['responses']['201']
>;
type ContactGetSuccess = ResponseBody<
  paths['/api/contacts/{address}']['get']['responses']['200']
>;
type ContactNotePutSuccess = ResponseBody<
  paths['/api/contacts/{address}/note']['put']['responses']['200']
>;
type ScreenerGetSuccess = ResponseBody<
  paths['/api/views/screener']['get']['responses']['200']
>;
type ViewCountsGetSuccess = ResponseBody<
  paths['/api/views/counts']['get']['responses']['200']
>;
type ScreenerAllowedGetSuccess = ResponseBody<
  paths['/api/views/screener/allowed']['get']['responses']['200']
>;
type DeniedSendersGetSuccess = ResponseBody<
  paths['/api/views/screener/denied']['get']['responses']['200']
>;
type UndoDenyPostSuccess = ResponseBody<
  paths['/api/screener/{address}/undo-deny']['post']['responses']['200']
>;
type ScreenerDecisionPostSuccess = ResponseBody<
  paths['/api/screener/decisions']['post']['responses']['200']
>;
type PileGetSuccess = ResponseBody<
  paths['/api/views/set-aside']['get']['responses']['200']
>;
type MailViewGetSuccess = ResponseBody<
  paths['/api/views/imbox']['get']['responses']['200']
>;
type ArchiveGetSuccess = ResponseBody<
  paths['/api/views/archive']['get']['responses']['200']
>;
type TrashGetSuccess = ResponseBody<
  paths['/api/views/trash']['get']['responses']['200']
>;
type SearchGetSuccess = ResponseBody<
  paths['/api/views/search']['get']['responses']['200']
>;
type LabelThreadsGetSuccess = ResponseBody<
  paths['/api/labels/{id}/threads']['get']['responses']['200']
>;
type LabelListSuccess = ResponseBody<
  paths['/api/labels']['get']['responses']['200']
>;
type LabelItemSuccess = ResponseBody<
  paths['/api/labels']['post']['responses']['201']
>;
type BatchAssignLabelSuccess = ResponseBody<
  paths['/api/threads/labels']['post']['responses']['200']
>;
type LabelRenameSuccess = ResponseBody<
  paths['/api/labels/{id}']['patch']['responses']['200']
>;
type BubbleUpGetSuccess = ResponseBody<
  paths['/api/views/bubble-up']['get']['responses']['200']
>;
type BubbleUpPostSuccess = ResponseBody<
  paths['/api/threads/{thread_id}/bubble-up']['post']['responses']['201']
>;
type BubbleUpDeleteSuccess = ResponseBody<
  paths['/api/threads/{thread_id}/bubble-up']['delete']['responses']['200']
>;
type ThreadVerbPostSuccess = ResponseBody<
  paths['/api/threads/{thread_id}/classify']['post']['responses']['200']
>;
type SpamThreadPostSuccess = ResponseBody<
  paths['/api/threads/{thread_id}/spam']['post']['responses']['200']
>;
type NotSpamThreadPostSuccess = ResponseBody<
  paths['/api/threads/{thread_id}/not-spam']['post']['responses']['200']
>;
type RestoreThreadPostSuccess = ResponseBody<
  paths['/api/threads/{thread_id}/restore']['post']['responses']['200']
>;
type DestroyThreadDeleteSuccess = ResponseBody<
  paths['/api/threads/{thread_id}/destroy']['delete']['responses']['200']
>;
type UndoPostSuccess = ResponseBody<
  paths['/api/undo/{id}']['post']['responses']['200']
>;
type AdminStatsGetSuccess = ResponseBody<
  paths['/api/admin/stats']['get']['responses']['200']
>;
type ScheduledSendsGetSuccess = ResponseBody<
  paths['/api/scheduled-sends']['get']['responses']['200']
>;
type ScheduledSendDeleteSuccess = ResponseBody<
  paths['/api/scheduled-sends/{scheduled_send_id}']['delete']['responses']['200']
>;
type WorkflowsGetSuccess = ResponseBody<
  paths['/api/workflows']['get']['responses']['200']
>;
type WorkflowPostSuccess = ResponseBody<
  paths['/api/workflows']['post']['responses']['201']
>;
type WorkflowPutSuccess = ResponseBody<
  paths['/api/workflows/{id}']['put']['responses']['200']
>;
type InvitePreviewSuccess = ResponseBody<
  paths['/api/invite/{token}']['get']['responses']['200']
>;
type InviteAcceptSuccess = ResponseBody<
  paths['/api/invite/{token}/accept']['post']['responses']['201']
>;
type GmailConnectSuccess = ResponseBody<
  paths['/api/provider-accounts/gmail/connect']['post']['responses']['200']
>;
type ProviderAccountSuccess = ResponseBody<
  paths['/api/provider-accounts/{id}/disconnect']['post']['responses']['200']
>;
type ProviderSyncStatusListSuccess = ResponseBody<
  paths['/api/provider-accounts/sync-status']['get']['responses']['200']
>;
type ProviderSyncTriggerSuccess = ResponseBody<
  paths['/api/provider-accounts/{id}/sync']['post']['responses']['200']
>;
type ProviderReimportSuccess = ResponseBody<
  paths['/api/provider-accounts/{id}/reimport']['post']['responses']['200']
>;
type SpeakeasyGetSuccess = ResponseBody<
  paths['/api/speakeasy']['get']['responses']['200']
>;
type SpeakeasyRotateSuccess = ResponseBody<
  paths['/api/speakeasy/rotate']['post']['responses']['200']
>;

type HailApiErrorBody<Status extends number> = Status extends 503
  ? ReadyzUnavailable
  : unknown;

export interface UserView {
  id: number;
  email: string;
  display_name: string | null;
  is_admin: boolean;
}

export interface UserEnvelope {
  user: UserView;
}

export interface AdminUsersResponse {
  users: UserView[];
}

export interface CreateAdminUserRequest {
  email: string;
  password: string;
  display_name?: string | null;
}

export interface CreateInviteRequest {
  email: string;
  display_name?: string | null;
}

export interface CreatedInviteResponse {
  email: string;
  display_name?: string | null;
  expires_at: string;
  invite_url: string;
}

export interface CreatedInviteEnvelope {
  invite: CreatedInviteResponse;
}

export type InvitePreview = InvitePreviewSuccess;
export type InviteAcceptResponse = InviteAcceptSuccess;
export type GmailConnectResponse = GmailConnectSuccess;
export type ProviderAccount = components['schemas']['ProviderAccountResponse'];
export type ProviderAccountResponse = ProviderAccountSuccess;
export type ProviderSyncEventSummary = components['schemas']['ProviderSyncEventSummary'];
export type ProviderSyncStatus = components['schemas']['ProviderSyncStatusResponse'];
export type ProviderSyncStatusListResponse = ProviderSyncStatusListSuccess;
export type ProviderSyncTriggerResponse = ProviderSyncTriggerSuccess;
export type ProviderReimportResponse = ProviderReimportSuccess;

export interface AcceptInviteRequest {
  password: string;
}

export interface ResetAdminUserPasswordRequest {
  password: string;
}

export interface AdminDomainsResponse {
  domains: string[];
}

export interface AdminDomainResponse {
  domain: string;
}

export interface AddAdminDomainRequest {
  domain: string;
}

export type AdminStatsResponse = AdminStatsGetSuccess;
export type AdminUserStats = components['schemas']['AdminUserStats'];

export interface LoginRequest {
  email: string;
  password: string;
}

export interface SetupState {
  wizard_active: boolean;
  reason?: 'config_admin_set' | 'admin_user_exists';
}

export interface SetupAdminRequest {
  email: string;
  password: string;
  display_name?: string | null;
  domain: string;
  bootstrap_token: string;
}

export type MailClassification = components['schemas']['MailClassification'];
export type MailViewKind = Extract<MailClassification, 'imbox' | 'feed' | 'papertrail'>;
export type DraftsViewKind = 'drafts';
export type TrashViewKind = 'trash';
export type ArchiveViewKind = 'archive';
export type PileViewKind = 'set-aside' | 'reply-later';
export type SearchScope = 'all' | 'mail' | 'notes' | 'clips';
export type SearchMailbox = 'all' | 'imbox' | 'feed' | 'papertrail' | 'archive' | 'trash' | 'drafts';
export type LabelResponse = components['schemas']['LabelResponse'];
export type FeedBlockedTracker = components['schemas']['FeedBlockedTrackerResponse'];
export type LabelItemResponse = LabelItemSuccess | LabelRenameSuccess | BatchAssignLabelSuccess;
export type CreateLabelRequest = components['schemas']['CreateLabelRequest'];
export type RenameLabelRequest = components['schemas']['RenameLabelRequest'];
export type AssignLabelNameRequest = components['schemas']['AssignLabelNameRequest'];
export type BatchAssignLabelRequest = components['schemas']['BatchAssignLabelRequest'];
export type LabelThreadItem = components['schemas']['LabelThreadItem'];
export type LabelThreadsResponse = LabelThreadsGetSuccess;
export type MailViewItem = components['schemas']['MailViewItem'];
export type MailViewResponse = MailViewGetSuccess;
export type ViewCountsResponse = ViewCountsGetSuccess;
export interface ImboxSectionedResponse {
  bubbled_up: MailViewItem[];
  new_for_you: MailViewItem[];
  previously_seen: MailViewItem[];
  new_count: number;
  previously_seen_total: number;
}
export type TrashViewResponse = TrashGetSuccess;
export type ArchiveViewResponse = ArchiveGetSuccess;
export type SearchResult = components['schemas']['SearchResult'];
export type MailSearchResult = Extract<SearchResult, { type: 'mail' }>;
export type ContactNoteSearchResult = Extract<SearchResult, { type: 'contact_note' }>;
export type SearchResponse = SearchGetSuccess;
export type LabelListResponse = LabelListSuccess;

export interface SearchParams {
  q: string;
  scope?: SearchScope;
  mailbox?: SearchMailbox;
  label_id?: number;
}

export type BlockedTracker = components['schemas']['BlockedTrackerResponse'];
export type ThreadParticipant = components['schemas']['Participant'];
export type ThreadMessage = components['schemas']['ThreadMessageResponse'];
export type ThreadNote = components['schemas']['ThreadNoteResponse'];
export type ThreadNotesResponse = NonNullable<ResponseBody<
  paths['/api/threads/{thread_id}/notes']['get']['responses']['200']
>>;
export type CreateThreadNoteRequest = components['schemas']['CreateThreadNoteRequest'];
export type ThreadViewResponse = ThreadGetSuccess;

export type PileItem = components['schemas']['PileItem'];
export type PileViewResponse = PileGetSuccess;
export type ContactNote = ContactNotePutSuccess;
export type ContactResponse = ContactGetSuccess;
export type PutContactNoteRequest = components['schemas']['PutNoteRequest'];
export type BubbleUpRequest = components['schemas']['BubbleUpRequest'];
export type BubbleUpResponse = BubbleUpPostSuccess;
export type BubbleUpViewItem = components['schemas']['BubbleUpViewItem'];
export type BubbleUpViewResponse = BubbleUpGetSuccess;
export type CancelBubbleUpResponse = BubbleUpDeleteSuccess;
export type UploadedBlob = components['schemas']['UploadedBlob'];
export interface AttachmentContext {
  thread_id: string;
  email_id: string;
  subject: string;
  from: string;
  received_at?: string | null;
  preview: string;
}
export interface AttachmentItem {
  blob_id: string;
  name: string;
  type: string;
  size: number;
  download_url: string;
  context: AttachmentContext;
}
export interface AttachmentsResponse {
  items: AttachmentItem[];
}

export type ComposeRequest = components['schemas']['ComposePayload'];
export type ReplyRequest = components['schemas']['ReplyPayload'];
export type ComposeResponse = components['schemas']['ComposeResponse'];
export type ScheduledSend = components['schemas']['ScheduledSendResponse'];
export type ScheduledSendsResponse = ScheduledSendsGetSuccess;
export type CancelScheduledSendResponse = ScheduledSendDeleteSuccess;
export type WorkflowRule = components['schemas']['WorkflowRule'];
export type WorkflowCondition = components['schemas']['WorkflowCondition'];
export type WorkflowConditionField = components['schemas']['WorkflowConditionField'];
export type WorkflowConditionOp = components['schemas']['WorkflowConditionOp'];
export type WorkflowAction = components['schemas']['WorkflowAction'];
export type WorkflowRulePayload = components['schemas']['WorkflowRulePayload'];
export type WorkflowRuleListResponse = WorkflowsGetSuccess;
export type WorkflowRuleResponse = WorkflowPostSuccess | WorkflowPutSuccess;
export type SpeakeasyState = components['schemas']['SpeakeasyState'];
export type SpeakeasyResponse = SpeakeasyGetSuccess;
export type RotateSpeakeasyRequest = components['schemas']['RotateSpeakeasyRequest'];
export type RotateSpeakeasyResponse = SpeakeasyRotateSuccess;
export type DraftRequest = components['schemas']['DraftPayload'];
export type DraftResponse = components['schemas']['DraftResponse'];
export type DraftDetails = DraftGetSuccess;

export type BlobUploadResponse = BlobUploadSuccess;

export type BlobUploadPart =
  | Blob
  | {
      blob: Blob;
      filename?: string;
    };

export type ScreenerDecision = 'approve' | 'deny';
export type ScreenerClassification = components['schemas']['MailClassification'];
export type ScreenerPendingSender = components['schemas']['ScreenerSender'];
export type ScreenerAllowedSender = components['schemas']['AllowedSender'];
export type ScreenerAllowedView = ScreenerAllowedGetSuccess;
export type DeniedSender = components['schemas']['DeniedSender'];
export type DeniedSendersResponse = DeniedSendersGetSuccess;
export interface UndoDenyRequest {
  classify_as?: ScreenerClassification | null;
}
export type UndoDenyResponse = UndoDenyPostSuccess;
export type ScreenerView = ScreenerGetSuccess;
export type ScreenerDecisionRequest = components['schemas']['DecisionRequest'];
export type UndoToken = components['schemas']['UndoToken'];
export type UndoResponse = UndoPostSuccess;
export type UndoableResponse = {
  undo?: UndoToken | null;
};
export type ScreenerDecisionResponse = ScreenerDecisionPostSuccess;
export type ThreadVerbResponse = ThreadVerbPostSuccess;
export type SpamThreadResponse = SpamThreadPostSuccess;
export type NotSpamThreadResponse = NotSpamThreadPostSuccess;
export type RestoreThreadResponse = RestoreThreadPostSuccess;
export type DestroyThreadResponse = DestroyThreadDeleteSuccess;

export class HailApiError<Status extends number = number> extends Error {
  readonly name = 'HailApiError';

  constructor(
    readonly status: Status,
    readonly body: HailApiErrorBody<Status>,
    readonly response: Response,
  ) {
    super(`hail API request failed with HTTP ${status}`);
  }
}

export class HailApiClient {
  readonly #baseUrl: URL;

  constructor(opts: { baseUrl: string }) {
    this.#baseUrl = new URL(opts.baseUrl);
  }

  async getHealthz(): Promise<HealthzSuccess> {
    const response = await this.#request('/healthz');

    if (response.status === 204) {
      return undefined as HealthzSuccess;
    }

    throw await this.#error(response);
  }

  async getReadyz(): Promise<ReadyzSuccess> {
    const response = await this.#request('/readyz');

    if (response.status === 200) {
      return undefined as ReadyzSuccess;
    }

    if (response.status === 503) {
      throw await this.#error<503>(response);
    }

    throw await this.#error(response);
  }

  async login(body: LoginRequest): Promise<UserEnvelope> {
    return this.#json<UserEnvelope>(
      await this.#request('/api/auth/login', {
        method: 'POST',
        body,
        mutating: true,
      }),
      200,
    );
  }

  async logout(): Promise<void> {
    await this.#empty(
      await this.#request('/api/auth/logout', {
        method: 'POST',
        mutating: true,
      }),
      204,
    );
  }

  async me(): Promise<UserEnvelope> {
    return this.#json<UserEnvelope>(await this.#request('/api/auth/me'), 200);
  }

  async getSetupState(): Promise<SetupState> {
    return this.#json<SetupState>(
      await this.#request('/api/setup/state'),
      200,
    );
  }

  async createSetupAdmin(body: SetupAdminRequest): Promise<UserEnvelope> {
    return this.#json<UserEnvelope>(
      await this.#request('/api/setup/admin', {
        method: 'POST',
        body,
        mutating: true,
      }),
      201,
    );
  }

  async listAdminUsers(): Promise<AdminUsersResponse> {
    return this.#json<AdminUsersResponse>(
      await this.#request('/api/admin/users'),
      200,
    );
  }

  async createAdminUser(body: CreateAdminUserRequest): Promise<UserEnvelope> {
    return this.#json<UserEnvelope>(
      await this.#request('/api/admin/users', {
        method: 'POST',
        body,
        mutating: true,
      }),
      201,
    );
  }

  async createInvite(body: CreateInviteRequest): Promise<CreatedInviteEnvelope> {
    return this.#json<CreatedInviteEnvelope>(
      await this.#request('/api/admin/invites', {
        method: 'POST',
        body,
        mutating: true,
      }),
      201,
    );
  }

  async getInvite(token: string): Promise<InvitePreview> {
    return this.#json<InvitePreview>(
      await this.#request(`/api/invite/${encodeURIComponent(token)}`),
      200,
    );
  }

  async acceptInvite(
    token: string,
    body: AcceptInviteRequest,
  ): Promise<InviteAcceptResponse> {
    return this.#json<InviteAcceptResponse>(
      await this.#request(`/api/invite/${encodeURIComponent(token)}/accept`, {
        method: 'POST',
        body,
        mutating: true,
      }),
      201,
    );
  }

  async connectGmail(): Promise<GmailConnectResponse> {
    return this.#json<GmailConnectResponse>(
      await this.#request('/api/provider-accounts/gmail/connect', {
        method: 'POST',
        mutating: true,
      }),
      200,
    );
  }

  async disconnectProviderAccount(
    id: number,
  ): Promise<ProviderAccountResponse> {
    return this.#json<ProviderAccountResponse>(
      await this.#request(
        `/api/provider-accounts/${encodeURIComponent(String(id))}/disconnect`,
        {
          method: 'POST',
          mutating: true,
        },
      ),
      200,
    );
  }

  async listProviderSyncStatuses(): Promise<ProviderSyncStatusListResponse> {
    return this.#json<ProviderSyncStatusListResponse>(
      await this.#request('/api/provider-accounts/sync-status'),
      200,
    );
  }

  async triggerProviderSync(id: number): Promise<ProviderSyncTriggerResponse> {
    return this.#json<ProviderSyncTriggerResponse>(
      await this.#request(
        `/api/provider-accounts/${encodeURIComponent(String(id))}/sync`,
        {
          method: 'POST',
          mutating: true,
        },
      ),
      200,
    );
  }

  async reimportProviderAccount(id: number): Promise<ProviderReimportResponse> {
    return this.#json<ProviderReimportResponse>(
      await this.#request(
        `/api/provider-accounts/${encodeURIComponent(String(id))}/reimport`,
        {
          method: 'POST',
          mutating: true,
        },
      ),
      200,
    );
  }

  async deleteAdminUser(userId: number): Promise<void> {
    await this.#empty(
      await this.#request(`/api/admin/users/${encodeURIComponent(String(userId))}`, {
        method: 'DELETE',
        mutating: true,
      }),
      204,
    );
  }

  async resetAdminUserPassword(
    userId: number,
    body: ResetAdminUserPasswordRequest,
  ): Promise<UserEnvelope> {
    return this.#json<UserEnvelope>(
      await this.#request(
        `/api/admin/users/${encodeURIComponent(String(userId))}/reset-password`,
        {
          method: 'POST',
          body,
          mutating: true,
        },
      ),
      200,
    );
  }

  async listAdminDomains(): Promise<AdminDomainsResponse> {
    return this.#json<AdminDomainsResponse>(
      await this.#request('/api/admin/domains'),
      200,
    );
  }

  async getAdminStats(): Promise<AdminStatsResponse> {
    return this.#json<AdminStatsResponse>(
      await this.#request('/api/admin/stats'),
      200,
    );
  }

  async addAdminDomain(
    body: AddAdminDomainRequest,
  ): Promise<AdminDomainResponse> {
    return this.#json<AdminDomainResponse>(
      await this.#request('/api/admin/domains', {
        method: 'POST',
        body,
        mutating: true,
      }),
      201,
    );
  }

  async deleteAdminDomain(domain: string): Promise<void> {
    await this.#empty(
      await this.#request(`/api/admin/domains/${encodeURIComponent(domain)}`, {
        method: 'DELETE',
        mutating: true,
      }),
      204,
    );
  }

  async getScreenerView(
    params: { cursor?: string; limit?: number } = {},
  ): Promise<ScreenerView> {
    const query = new URLSearchParams();
    if (params.cursor) {
      query.set('cursor', params.cursor);
    }
    if (params.limit !== undefined) {
      query.set('limit', String(params.limit));
    }
    const suffix = query.toString();

    return this.#json<ScreenerView>(
      await this.#request(`/api/views/screener${suffix ? `?${suffix}` : ''}`),
      200,
    );
  }

  async getViewCounts(): Promise<ViewCountsResponse> {
    return this.#json<ViewCountsResponse>(
      await this.#request('/api/views/counts'),
      200,
    );
  }

  async getScreenerAllowedView(): Promise<ScreenerAllowedView> {
    return this.#json<ScreenerAllowedView>(
      await this.#request('/api/views/screener/allowed'),
      200,
    );
  }

  async getDeniedSenders(): Promise<DeniedSendersResponse> {
    return this.#json<DeniedSendersResponse>(
      await this.#request('/api/views/screener/denied'),
      200,
    );
  }

  async getImbox(): Promise<MailViewResponse> {
    return this.#json<MailViewResponse>(
      await this.#request('/api/views/imbox'),
      200,
    );
  }

  async getImboxSectioned(): Promise<ImboxSectionedResponse> {
    return this.#json<ImboxSectionedResponse>(
      await this.#request('/api/views/imbox/sectioned'),
      200,
    );
  }

  async getFeed(): Promise<MailViewResponse> {
    return this.#json<MailViewResponse>(
      await this.#request('/api/views/feed'),
      200,
    );
  }

  async getPapertrail(): Promise<MailViewResponse> {
    return this.#json<MailViewResponse>(
      await this.#request('/api/views/papertrail'),
      200,
    );
  }

  async getDrafts(): Promise<MailViewResponse> {
    return this.#json<MailViewResponse>(
      await this.#request('/api/views/drafts'),
      200,
    );
  }

  async getArchiveView(): Promise<ArchiveViewResponse> {
    return this.#json<ArchiveViewResponse>(
      await this.#request('/api/views/archive'),
      200,
    );
  }

  async getSpamView(): Promise<MailViewResponse> {
    return this.#json<MailViewResponse>(
      await this.#request('/api/views/spam'),
      200,
    );
  }

  async getTrash(): Promise<TrashViewResponse> {
    return this.#json<TrashViewResponse>(
      await this.#request('/api/views/trash'),
      200,
    );
  }

  async search(params: SearchParams): Promise<SearchResponse> {
    const query = new URLSearchParams({ q: params.q });
    if (params.scope) {
      query.set('scope', params.scope);
    }
    if (params.mailbox && params.mailbox !== 'all') {
      query.set('mailbox', params.mailbox);
    }
    if (params.label_id !== undefined) {
      query.set('label_id', String(params.label_id));
    }

    return this.#json<SearchResponse>(
      await this.#request(`/api/views/search?${query.toString()}`),
      200,
    );
  }

  async getLabelThreads(
    labelId: number,
    params: { cursor?: string; limit?: number } = {},
  ): Promise<LabelThreadsResponse> {
    const query = new URLSearchParams();
    if (params.cursor) {
      query.set('cursor', params.cursor);
    }
    if (params.limit !== undefined) {
      query.set('limit', String(params.limit));
    }
    const suffix = query.toString();

    return this.#json<LabelThreadsResponse>(
      await this.#request(
        `/api/labels/${encodeURIComponent(String(labelId))}/threads${suffix ? `?${suffix}` : ''}`,
      ),
      200,
    );
  }

  async listLabels(): Promise<LabelListResponse> {
    return this.#json<LabelListResponse>(
      await this.#request('/api/labels'),
      200,
    );
  }

  async createLabel(body: CreateLabelRequest): Promise<LabelItemResponse> {
    return this.#json<LabelItemResponse>(
      await this.#request('/api/labels', {
        method: 'POST',
        body,
        mutating: true,
      }),
      201,
    );
  }

  async renameLabel(
    labelId: number,
    body: RenameLabelRequest,
  ): Promise<LabelItemResponse> {
    return this.#json<LabelItemResponse>(
      await this.#request(`/api/labels/${encodeURIComponent(String(labelId))}`, {
        method: 'PATCH',
        body,
        mutating: true,
      }),
      200,
    );
  }

  async deleteLabel(labelId: number): Promise<void> {
    await this.#empty(
      await this.#request(`/api/labels/${encodeURIComponent(String(labelId))}`, {
        method: 'DELETE',
        mutating: true,
      }),
      204,
    );
  }

  async assignLabelToThread(
    threadId: string,
    labelId: number,
  ): Promise<LabelItemResponse> {
    return this.#json<LabelItemResponse>(
      await this.#request(
        `/api/threads/${encodeURIComponent(threadId)}/labels/${encodeURIComponent(String(labelId))}`,
        {
          method: 'POST',
          mutating: true,
        },
      ),
      200,
    );
  }

  async assignLabelNameToThread(
    threadId: string,
    body: AssignLabelNameRequest,
  ): Promise<LabelItemResponse> {
    return this.#json<LabelItemResponse>(
      await this.#request(`/api/threads/${encodeURIComponent(threadId)}/labels`, {
        method: 'POST',
        body,
        mutating: true,
      }),
      200,
    );
  }

  async assignLabelToThreads(
    body: BatchAssignLabelRequest,
  ): Promise<LabelItemResponse> {
    return this.#json<LabelItemResponse>(
      await this.#request('/api/threads/labels', {
        method: 'POST',
        body,
        mutating: true,
      }),
      200,
    );
  }

  async removeLabelFromThread(threadId: string, labelId: number): Promise<void> {
    await this.#empty(
      await this.#request(
        `/api/threads/${encodeURIComponent(threadId)}/labels/${encodeURIComponent(String(labelId))}`,
        {
          method: 'DELETE',
          mutating: true,
        },
      ),
      204,
    );
  }

  async getThread(threadId: string): Promise<ThreadViewResponse> {
    return this.#json<ThreadViewResponse>(
      await this.#request(`/api/threads/${encodeURIComponent(threadId)}`),
      200,
    );
  }

  async listAttachments(limit = 50): Promise<AttachmentsResponse> {
    const query = new URLSearchParams({ limit: String(limit) });
    return this.#json<AttachmentsResponse>(
      await this.#request(`/api/attachments?${query.toString()}`),
      200,
    );
  }

  async getThreadNotes(threadId: string): Promise<ThreadNotesResponse> {
    return this.#json<ThreadNotesResponse>(
      await this.#request(`/api/threads/${encodeURIComponent(threadId)}/notes`),
      200,
    );
  }

  async createThreadNote(
    threadId: string,
    body: CreateThreadNoteRequest,
  ): Promise<ThreadNote> {
    return this.#json<ThreadNote>(
      await this.#request(`/api/threads/${encodeURIComponent(threadId)}/notes`, {
        method: 'POST',
        body,
        mutating: true,
      }),
      201,
    );
  }

  async deleteThreadNote(threadId: string, noteId: number): Promise<void> {
    await this.#empty(
      await this.#request(
        `/api/threads/${encodeURIComponent(threadId)}/notes/${encodeURIComponent(String(noteId))}`,
        {
          method: 'DELETE',
          mutating: true,
        },
      ),
      204,
    );
  }

  async getSetAside(): Promise<PileViewResponse> {
    return this.#json<PileViewResponse>(
      await this.#request('/api/views/set-aside'),
      200,
    );
  }

  async getReplyLater(): Promise<PileViewResponse> {
    return this.#json<PileViewResponse>(
      await this.#request('/api/views/reply-later'),
      200,
    );
  }

  async getBubbleUps(): Promise<BubbleUpViewResponse> {
    return this.#json<BubbleUpViewResponse>(
      await this.#request('/api/views/bubble-up'),
      200,
    );
  }

  async getSpeakeasy(): Promise<SpeakeasyResponse> {
    return this.#json<SpeakeasyResponse>(
      await this.#request('/api/speakeasy'),
      200,
    );
  }

  async rotateSpeakeasy(
    body: RotateSpeakeasyRequest = { acknowledge_bypass_secret: true },
  ): Promise<RotateSpeakeasyResponse> {
    return this.#json<RotateSpeakeasyResponse>(
      await this.#request('/api/speakeasy/rotate', {
        method: 'POST',
        body,
        mutating: true,
      }),
      200,
    );
  }

  async decideScreener(
    body: ScreenerDecisionRequest,
  ): Promise<ScreenerDecisionResponse> {
    return this.#json<ScreenerDecisionResponse>(
      await this.#request('/api/screener/decisions', {
        method: 'POST',
        body,
        mutating: true,
      }),
      200,
    );
  }

  async undoDeny(
    address: string,
    body?: UndoDenyRequest,
  ): Promise<UndoDenyResponse> {
    const requestBody = body ?? { classify_as: null };
    return this.#json<UndoDenyResponse>(
      await this.#request(
        `/api/screener/${encodeURIComponent(address)}/undo-deny`,
        {
          method: 'POST',
          body: requestBody,
          mutating: true,
        },
      ),
      200,
    );
  }

  async classifyThread(
    threadId: string,
    to: MailClassification,
  ): Promise<ThreadVerbResponse> {
    return this.#json<ThreadVerbResponse>(
      await this.#request(`/api/threads/${encodeURIComponent(threadId)}/classify`, {
        method: 'POST',
        body: { to },
        mutating: true,
      }),
      200,
    );
  }

  async moveThread(
    threadId: string,
    to: 'imbox' | 'feed' | 'papertrail',
  ): Promise<ThreadVerbResponse> {
    return this.classifyThread(threadId, to);
  }

  async archiveThread(threadId: string): Promise<ThreadVerbResponse> {
    return this.#json<ThreadVerbResponse>(
      await this.#request(`/api/threads/${encodeURIComponent(threadId)}/archive`, {
        method: 'POST',
        mutating: true,
      }),
      200,
    );
  }

  async trashThread(threadId: string): Promise<ThreadVerbResponse> {
    return this.#json<ThreadVerbResponse>(
      await this.#request(`/api/threads/${encodeURIComponent(threadId)}/trash`, {
        method: 'POST',
        mutating: true,
      }),
      200,
    );
  }

  async spamThread(threadId: string): Promise<SpamThreadResponse> {
    return this.#json<SpamThreadResponse>(
      await this.#request(`/api/threads/${encodeURIComponent(threadId)}/spam`, {
        method: 'POST',
        mutating: true,
      }),
      200,
    );
  }

  async notSpamThread(threadId: string): Promise<NotSpamThreadResponse> {
    return this.#json<NotSpamThreadResponse>(
      await this.#request(`/api/threads/${encodeURIComponent(threadId)}/not-spam`, {
        method: 'POST',
        mutating: true,
      }),
      200,
    );
  }

  async restoreThread(threadId: string): Promise<RestoreThreadResponse> {
    return this.#json<RestoreThreadResponse>(
      await this.#request(`/api/threads/${encodeURIComponent(threadId)}/restore`, {
        method: 'POST',
        mutating: true,
      }),
      200,
    );
  }

  async destroyThread(threadId: string): Promise<DestroyThreadResponse> {
    return this.#json<DestroyThreadResponse>(
      await this.#request(`/api/threads/${encodeURIComponent(threadId)}/destroy`, {
        method: 'DELETE',
        mutating: true,
      }),
      200,
    );
  }

  async markThread(threadId: string, read: boolean): Promise<void> {
    await this.#empty(
      await this.#request(`/api/threads/${encodeURIComponent(threadId)}/mark`, {
        method: 'POST',
        body: { read },
        mutating: true,
      }),
      204,
    );
  }

  async undo(id: string): Promise<UndoResponse> {
    return this.#json<UndoResponse>(
      await this.#request(`/api/undo/${encodeURIComponent(id)}`, {
        method: 'POST',
        mutating: true,
      }),
      200,
    );
  }

  async getContact(address: string): Promise<ContactResponse> {
    return this.#json<ContactResponse>(
      await this.#request(`/api/contacts/${encodeURIComponent(address)}`),
      200,
    );
  }

  async putContactNote(
    address: string,
    body: PutContactNoteRequest,
  ): Promise<ContactNote> {
    return this.#json<ContactNote>(
      await this.#request(
        `/api/contacts/${encodeURIComponent(address)}/note`,
        {
          method: 'PUT',
          body,
          mutating: true,
        },
      ),
      200,
    );
  }

  async deleteContactNote(address: string): Promise<void> {
    await this.#empty(
      await this.#request(`/api/contacts/${encodeURIComponent(address)}/note`, {
        method: 'DELETE',
        mutating: true,
      }),
      204,
    );
  }

  async setAsideThread(threadId: string): Promise<ThreadVerbResponse> {
    return this.#json<ThreadVerbResponse>(
      await this.#request(`/api/threads/${encodeURIComponent(threadId)}/set-aside`, {
        method: 'POST',
        mutating: true,
      }),
      200,
    );
  }

  async replyLaterThread(threadId: string): Promise<ThreadVerbResponse> {
    return this.#json<ThreadVerbResponse>(
      await this.#request(`/api/threads/${encodeURIComponent(threadId)}/reply-later`, {
        method: 'POST',
        mutating: true,
      }),
      200,
    );
  }

  async bubbleUpThread(
    threadId: string,
    body: BubbleUpRequest,
  ): Promise<BubbleUpResponse> {
    return this.#json<BubbleUpResponse>(
      await this.#request(
        `/api/threads/${encodeURIComponent(threadId)}/bubble-up`,
        {
          method: 'POST',
          body,
          mutating: true,
        },
      ),
      201,
    );
  }

  async cancelBubbleUp(threadId: string): Promise<CancelBubbleUpResponse> {
    return this.#json<CancelBubbleUpResponse>(
      await this.#request(
        `/api/threads/${encodeURIComponent(threadId)}/bubble-up`,
        {
          method: 'DELETE',
          mutating: true,
        },
      ),
      200,
    );
  }

  async sendCompose(body: ComposeRequest): Promise<ComposeResponse> {
    return this.#json<ComposeResponse>(
      await this.#request('/api/compose', {
        method: 'POST',
        body,
        mutating: true,
      }),
      body.send_at ? 201 : 200,
    );
  }

  async sendReply(
    threadId: string,
    body: ReplyRequest,
  ): Promise<ComposeResponse> {
    return this.#json<ComposeResponse>(
      await this.#request(`/api/threads/${encodeURIComponent(threadId)}/reply`, {
        method: 'POST',
        body,
        mutating: true,
      }),
      body.send_at ? 201 : 200,
    );
  }

  async listScheduledSends(): Promise<ScheduledSendsResponse> {
    return this.#json<ScheduledSendsResponse>(
      await this.#request('/api/scheduled-sends'),
      200,
    );
  }

  async cancelScheduledSend(
    scheduledSendId: number,
  ): Promise<CancelScheduledSendResponse> {
    return this.#json<CancelScheduledSendResponse>(
      await this.#request(
        `/api/scheduled-sends/${encodeURIComponent(String(scheduledSendId))}`,
        {
          method: 'DELETE',
          mutating: true,
        },
      ),
      200,
    );
  }

  async createDraft(body: DraftRequest): Promise<DraftResponse> {
    return this.#json<DraftResponse>(
      await this.#request('/api/drafts', {
        method: 'POST',
        body,
        mutating: true,
      }),
      201,
    );
  }

  async getDraft(draftId: string): Promise<DraftDetails> {
    return this.#json<DraftDetails>(
      await this.#request(`/api/drafts/${encodeURIComponent(draftId)}`),
      200,
    );
  }

  async updateDraft(
    draftId: string,
    body: DraftRequest,
  ): Promise<DraftResponse> {
    return this.#json<DraftResponse>(
      await this.#request(`/api/drafts/${encodeURIComponent(draftId)}`, {
        method: 'PATCH',
        body,
        mutating: true,
      }),
      200,
    );
  }

  async deleteDraft(draftId: string): Promise<void> {
    await this.#empty(
      await this.#request(`/api/drafts/${encodeURIComponent(draftId)}`, {
        method: 'DELETE',
        mutating: true,
      }),
      204,
    );
  }

  async listWorkflows(): Promise<WorkflowRuleListResponse> {
    return this.#json<WorkflowRuleListResponse>(
      await this.#request('/api/workflows'),
      200,
    );
  }

  async createWorkflow(body: WorkflowRulePayload): Promise<WorkflowRuleResponse> {
    return this.#json<WorkflowRuleResponse>(
      await this.#request('/api/workflows', {
        method: 'POST',
        body,
        mutating: true,
      }),
      201,
    );
  }

  async updateWorkflow(
    id: number,
    body: WorkflowRulePayload,
  ): Promise<WorkflowRuleResponse> {
    return this.#json<WorkflowRuleResponse>(
      await this.#request(`/api/workflows/${encodeURIComponent(String(id))}`, {
        method: 'PUT',
        body,
        mutating: true,
      }),
      200,
    );
  }

  async deleteWorkflow(id: number): Promise<void> {
    await this.#empty(
      await this.#request(`/api/workflows/${encodeURIComponent(String(id))}`, {
        method: 'DELETE',
        mutating: true,
      }),
      204,
    );
  }

  async uploadBlob(file: BlobUploadPart): Promise<UploadedBlob> {
    const { blobs } = await this.uploadBlobs([file]);
    const [blob] = blobs;
    if (!blob) {
      throw new Error('hail API returned no blob for upload');
    }
    return blob;
  }

  async uploadBlobs(files: Iterable<BlobUploadPart>): Promise<BlobUploadResponse> {
    const formData = new FormData();
    for (const file of files) {
      if (file instanceof Blob) {
        formData.append('file', file);
      } else if (file.filename) {
        formData.append('file', file.blob, file.filename);
      } else {
        formData.append('file', file.blob);
      }
    }

    return this.#json<BlobUploadResponse>(
      await this.#request('/api/blobs', {
        method: 'POST',
        body: formData,
        mutating: true,
      }),
      201,
    );
  }

  async #json<T>(response: Response, expectedStatus: number): Promise<T> {
    if (response.status !== expectedStatus) {
      throw await this.#error(response);
    }

    return (await readResponseBody(response)) as T;
  }

  async #empty(response: Response, expectedStatus: number): Promise<void> {
    if (response.status !== expectedStatus) {
      throw await this.#error(response);
    }
  }

  async #request(
    pathname: string,
    opts: {
      method?: string;
      body?: unknown;
      mutating?: boolean;
    } = {},
  ): Promise<Response> {
    const url = new URL(pathname, this.#baseUrl);
    const headers = new Headers({
      accept: 'application/json',
    });
    let body: BodyInit | undefined;

    if (opts.mutating) {
      headers.set('X-Hail-Request', '1');
    }

    if (opts.body instanceof FormData) {
      body = opts.body;
    } else if (opts.body !== undefined) {
      headers.set('content-type', 'application/json');
      body = JSON.stringify(opts.body);
    }

    return fetch(url, {
      method: opts.method ?? 'GET',
      credentials: 'include',
      headers,
      body,
    });
  }

  async #error<Status extends number = number>(
    response: Response,
  ): Promise<HailApiError<Status>> {
    return new HailApiError(
      response.status as Status,
      (await readResponseBody(response)) as HailApiErrorBody<Status>,
      response,
    );
  }
}

async function readResponseBody(response: Response): Promise<unknown> {
  if (response.status === 204) {
    return undefined;
  }

  const text = await response.text();
  if (text.length === 0) {
    return undefined;
  }

  const contentType = response.headers.get('content-type') ?? '';
  if (contentType.includes('application/json')) {
    return JSON.parse(text);
  }

  return text;
}
