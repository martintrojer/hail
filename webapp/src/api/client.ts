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
}

export type MailClassification = 'imbox' | 'feed' | 'papertrail';
export type MailViewKind = MailClassification;
export type PileViewKind = 'set-aside' | 'reply-later';
export type SearchScope = 'all' | 'mail' | 'notes' | 'clips';

export interface MailViewItem {
  thread_id: string;
  email_id: string;
  from: string;
  subject: string;
  preview: string;
  received_at: string | null;
  unread: boolean;
  classification: MailClassification;
}

export interface MailViewResponse {
  items: MailViewItem[];
  next_cursor: string | null;
}

export interface MailSearchResult {
  type: 'mail';
  thread_id: string;
  email_id: string;
  from: string;
  subject: string;
  preview: string;
  received_at: string | null;
}

export interface ContactNoteSearchResult {
  type: 'contact_note';
  address: string;
  markdown: string;
  updated_at: string;
}

export type SearchResult = MailSearchResult | ContactNoteSearchResult;

export interface SearchResponse {
  results: SearchResult[];
}

export interface SearchParams {
  q: string;
  scope?: SearchScope;
}

export type BlockedTracker = components['schemas']['BlockedTrackerResponse'];
export type ThreadParticipant = components['schemas']['Participant'];
export type ThreadMessage = components['schemas']['ThreadMessageResponse'];
export type ThreadViewResponse = ThreadGetSuccess;

export interface PileItem {
  thread_id: string;
  position: number;
  added_at: string;
  preview?: unknown;
}

export interface PileViewResponse {
  items: PileItem[];
}

export interface ContactNote {
  markdown: string;
  updated_at: string;
}

export interface ContactResponse {
  address: string;
  note: ContactNote | null;
  threads: unknown[];
}

export interface PutContactNoteRequest {
  markdown: string;
}

export interface BubbleUpRequest {
  at: string;
}

export interface BubbleUpResponse {
  bubble_id: number;
  surface_at: string;
}

export interface UploadedBlob {
  blob_id: string;
  size: number;
  type: string;
}

export type ComposeRequest = components['schemas']['ComposePayload'];
export type ReplyRequest = components['schemas']['ReplyPayload'];
export type ComposeResponse = components['schemas']['ComposeResponse'];
export type DraftRequest = components['schemas']['DraftPayload'];
export type DraftResponse = components['schemas']['DraftResponse'];

export interface BlobUploadResponse {
  blobs: UploadedBlob[];
}

export type BlobUploadPart =
  | Blob
  | {
      blob: Blob;
      filename?: string;
    };

export type ScreenerDecision = 'approve' | 'deny';
export type ScreenerClassification = 'imbox' | 'feed' | 'papertrail';

export interface ScreenerPendingSender {
  sender: string;
  first_seen_at: string;
  message_count: number;
  latest_preview?: unknown;
  [key: string]: unknown;
}

/**
 * TODO(spa-api-client): replace this hand-written shape once hail-api exports
 * /api/views/screener in OpenAPI. The SPA must treat this as server-shaped data.
 */
export interface ScreenerView {
  senders: ScreenerPendingSender[];
  [key: string]: unknown;
}

/**
 * TODO(spa-api-client): replace this hand-written request once hail-api exports
 * /api/screener/decisions in OpenAPI.
 */
export interface ScreenerDecisionRequest {
  sender: string;
  decision: ScreenerDecision;
  classify_as?: ScreenerClassification;
  apply_to_history: boolean;
}

export interface UndoToken {
  id: string;
  action: string;
  expires_at: string;
}

export interface UndoResponse {
  id: string;
  action: string;
}

export type UndoableResponse = {
  undo?: UndoToken | null;
};

export type ScreenerDecisionResponse = UndoableResponse & {
  sender: string;
  decision: ScreenerDecision;
  classify_as?: ScreenerClassification | null;
};

export type ThreadVerbResponse = UndoableResponse;

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

  async getScreenerView(): Promise<ScreenerView> {
    return this.#json<ScreenerView>(
      await this.#request('/api/views/screener'),
      200,
    );
  }

  async getImbox(): Promise<MailViewResponse> {
    return this.#json<MailViewResponse>(
      await this.#request('/api/views/imbox'),
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

  async search(params: SearchParams): Promise<SearchResponse> {
    const query = new URLSearchParams({ q: params.q });
    if (params.scope) {
      query.set('scope', params.scope);
    }

    return this.#json<SearchResponse>(
      await this.#request(`/api/views/search?${query.toString()}`),
      200,
    );
  }

  async getThread(threadId: string): Promise<ThreadViewResponse> {
    return this.#json<ThreadViewResponse>(
      await this.#request(`/api/threads/${encodeURIComponent(threadId)}`),
      200,
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
