import { afterEach, describe, expect, it, vi } from 'vitest';
import {
  HailApiClient,
  HailApiError,
  type BlobUploadResponse,
  type BubbleUpRequest,
  type ComposeRequest,
  type CreateThreadNoteRequest,
  type DraftRequest,
  type PutContactNoteRequest,
  type ReplyRequest,
  type ScreenerDecisionRequest,
} from './client';

const client = new HailApiClient({ baseUrl: 'http://localhost' });

function jsonResponse(status: number, body: unknown) {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

function textResponse(status: number, body: string) {
  return new Response(body, {
    status,
    headers: { 'content-type': 'text/plain' },
  });
}

function emptyResponse(status: number) {
  return new Response(null, { status });
}

afterEach(() => {
  vi.restoreAllMocks();
});

describe('HailApiClient error body contract', () => {
  it('parses JSON error bodies into HailApiError.body', async () => {
    vi.spyOn(globalThis, 'fetch').mockResolvedValueOnce(
      jsonResponse(400, {
        error: 'invalid_send_at',
        detail: 'send_at must be in the future',
      }),
    );

    await expectHailApiError(client.sendCompose(composeRequest()), {
      status: 400,
      body: {
        error: 'invalid_send_at',
        detail: 'send_at must be in the future',
      },
    });
  });

  it('returns text error bodies without attempting JSON parsing', async () => {
    vi.spyOn(globalThis, 'fetch').mockResolvedValueOnce(
      textResponse(500, 'upstream unavailable'),
    );

    await expectHailApiError(client.me(), {
      status: 500,
      body: 'upstream unavailable',
    });
  });

  it('returns undefined for empty error bodies', async () => {
    vi.spyOn(globalThis, 'fetch').mockResolvedValueOnce(emptyResponse(401));

    await expectHailApiError(client.me(), {
      status: 401,
      body: undefined,
    });
  });
});

describe('HailApiClient GET request contract', () => {
  it('sends credentials and encoded query parameters for search', async () => {
    const fetchSpy = vi
      .spyOn(globalThis, 'fetch')
      .mockResolvedValueOnce(jsonResponse(200, { results: [] }));

    await client.search({
      q: 'from:alice+bob@example.org / tag?',
      scope: 'clips',
      mailbox: 'papertrail',
    });

    expect(fetchSpy.mock.calls[0]?.[0]).toEqual(
      new URL(
        'http://localhost/api/views/search?q=from%3Aalice%2Bbob%40example.org+%2F+tag%3F&scope=clips&mailbox=papertrail',
      ),
    );
    expectGetRequest(fetchSpy.mock.calls[0]?.[1]);
  });

  it('sends credentials and query limit for attachment listing', async () => {
    const fetchSpy = vi
      .spyOn(globalThis, 'fetch')
      .mockResolvedValueOnce(jsonResponse(200, { items: [] }));

    await client.listAttachments(75);

    expect(fetchSpy.mock.calls[0]?.[0]).toEqual(
      new URL('http://localhost/api/attachments?limit=75'),
    );
    expectGetRequest(fetchSpy.mock.calls[0]?.[1]);
  });

  it('sends credentials and encoded ids for thread lookups', async () => {
    const fetchSpy = vi
      .spyOn(globalThis, 'fetch')
      .mockResolvedValueOnce(jsonResponse(200, threadResponse()));

    await client.getThread('thread/with spaces?and#hash');

    expect(fetchSpy.mock.calls[0]?.[0]).toEqual(
      new URL('http://localhost/api/threads/thread%2Fwith%20spaces%3Fand%23hash'),
    );
    expectGetRequest(fetchSpy.mock.calls[0]?.[1]);
  });

  it('sends credentials and encoded ids for contact lookups', async () => {
    const fetchSpy = vi.spyOn(globalThis, 'fetch').mockResolvedValueOnce(
      jsonResponse(200, {
        address: 'person+tag@example.org / team',
        note: null,
        threads: [],
      }),
    );

    await client.getContact('person+tag@example.org / team');

    expect(fetchSpy.mock.calls[0]?.[0]).toEqual(
      new URL(
        'http://localhost/api/contacts/person%2Btag%40example.org%20%2F%20team',
      ),
    );
    expectGetRequest(fetchSpy.mock.calls[0]?.[1]);
  });
  it('sends credentials for Bubble Up view lookups', async () => {
    const fetchSpy = vi
      .spyOn(globalThis, 'fetch')
      .mockResolvedValueOnce(jsonResponse(200, { items: [] }));

    await client.getBubbleUps();

    expect(fetchSpy.mock.calls[0]?.[0]).toEqual(
      new URL('http://localhost/api/views/bubble-up'),
    );
    expectGetRequest(fetchSpy.mock.calls[0]?.[1]);
  });
  it('sends credentials for scheduled-send list lookups', async () => {
    const fetchSpy = vi
      .spyOn(globalThis, 'fetch')
      .mockResolvedValueOnce(jsonResponse(200, []));

    await client.listScheduledSends();

    expect(fetchSpy.mock.calls[0]?.[0]).toEqual(
      new URL('http://localhost/api/scheduled-sends'),
    );
    expectGetRequest(fetchSpy.mock.calls[0]?.[1]);
  });
});

describe('HailApiClient provider account contract', () => {
  it('starts Gmail OAuth with a mutating no-body request', async () => {
    const fetchSpy = vi.spyOn(globalThis, 'fetch').mockResolvedValueOnce(
      jsonResponse(200, {
        authorization_url: 'https://accounts.google.test/oauth',
        scopes: ['https://www.googleapis.com/auth/gmail.readonly'],
      }),
    );

    await expect(client.connectGmail()).resolves.toMatchObject({
      authorization_url: 'https://accounts.google.test/oauth',
    });

    expect(fetchSpy.mock.calls[0]?.[0]).toEqual(
      new URL('http://localhost/api/provider-accounts/gmail/connect'),
    );
    expectMutatingNoBodyRequest(fetchSpy.mock.calls[0]?.[1], 'POST');
  });

  it('disconnects provider accounts with encoded ids', async () => {
    const fetchSpy = vi.spyOn(globalThis, 'fetch').mockResolvedValueOnce(
      jsonResponse(200, {
        id: 42,
        provider_kind: 'gmail',
        provider_account_id: 'reader@gmail.com',
        provider_email: 'reader@gmail.com',
        display_email: 'reader@gmail.com',
        granted_scopes: ['https://www.googleapis.com/auth/gmail.readonly'],
        sync_status: 'disconnected',
        cached_access_token_expires_at: null,
        last_profile_history_id: '12345',
      }),
    );

    await expect(client.disconnectProviderAccount(42)).resolves.toMatchObject({
      id: 42,
      sync_status: 'disconnected',
    });

    expect(fetchSpy.mock.calls[0]?.[0]).toEqual(
      new URL('http://localhost/api/provider-accounts/42/disconnect'),
    );
    expectMutatingNoBodyRequest(fetchSpy.mock.calls[0]?.[1], 'POST');
  });
});

describe('HailApiClient blob upload contract', () => {
  it('uploads blobs as FormData without forcing a JSON content type', async () => {
    const response: BlobUploadResponse = {
      blobs: [{ blob_id: 'blob-1', size: 5, type: 'text/plain' }],
    };
    const fetchSpy = vi
      .spyOn(globalThis, 'fetch')
      .mockResolvedValueOnce(jsonResponse(201, response));
    const blob = new Blob(['hello'], { type: 'text/plain' });

    await expect(
      client.uploadBlobs([{ blob, filename: 'hello world.txt' }]),
    ).resolves.toEqual(response);

    expect(fetchSpy.mock.calls[0]?.[0]).toEqual(
      new URL('http://localhost/api/blobs'),
    );
    expect(fetchSpy.mock.calls[0]?.[1]?.method).toBe('POST');
    expect(fetchSpy.mock.calls[0]?.[1]?.credentials).toBe('include');
    expect(fetchSpy.mock.calls[0]?.[1]?.body).toBeInstanceOf(FormData);
    expect(
      (fetchSpy.mock.calls[0]?.[1]?.body as FormData).getAll('file'),
    ).toHaveLength(1);

    const headers = expectHeaders(fetchSpy.mock.calls[0]?.[1]);
    expect(headers.get('X-Hail-Request')).toBe('1');
    expect(headers.get('accept')).toBe('application/json');
    expect(headers.has('content-type')).toBe(false);
  });
});

describe('HailApiClient compose status contract', () => {
  it('expects immediate compose sends to return 200 sent', async () => {
    vi.spyOn(globalThis, 'fetch').mockResolvedValueOnce(
      jsonResponse(200, {
        status: 'sent',
        email_id: 'draft-1',
        submission_id: 'submission-1',
      }),
    );

    await expect(client.sendCompose(composeRequest())).resolves.toMatchObject({
      status: 'sent',
      email_id: 'draft-1',
    });
  });

  it('expects send-later compose sends to return 201 pending', async () => {
    vi.spyOn(globalThis, 'fetch').mockResolvedValueOnce(
      jsonResponse(201, {
        status: 'pending',
        scheduled_send_id: 1,
        draft_email_id: 'draft-1',
      }),
    );

    await expect(
      client.sendCompose(composeRequest({ send_at: '2026-05-23T13:00:00Z' })),
    ).resolves.toMatchObject({
      status: 'pending',
      scheduled_send_id: 1,
    });
  });

  it('surfaces invalid send_at errors instead of accepting immediate-send fallback', async () => {
    vi.spyOn(globalThis, 'fetch').mockResolvedValueOnce(
      jsonResponse(400, {
        error: 'invalid_send_at',
      }),
    );

    await expect(
      client.sendCompose(composeRequest({ send_at: '2000-01-01T00:00:00Z' })),
    ).rejects.toMatchObject({
      status: 400,
      body: { error: 'invalid_send_at' },
    });
  });
});

describe('HailApiClient compose mutating requests', () => {
  it('sends CSRF header and credentials for sendCompose', async () => {
    const body = composeRequest();
    const fetchSpy = vi.spyOn(globalThis, 'fetch').mockResolvedValueOnce(
      jsonResponse(200, {
        status: 'sent',
        email_id: 'draft-1',
        submission_id: 'submission-1',
      }),
    );

    await client.sendCompose(body);

    expect(fetchSpy.mock.calls[0]?.[0]).toEqual(
      new URL('http://localhost/api/compose'),
    );
    expectMutatingJsonRequest(fetchSpy.mock.calls[0]?.[1], 'POST', body);
  });

  it('sends CSRF header and credentials for sendReply', async () => {
    const body = replyRequest();
    const fetchSpy = vi.spyOn(globalThis, 'fetch').mockResolvedValueOnce(
      jsonResponse(200, {
        status: 'sent',
        email_id: 'draft-1',
        submission_id: 'submission-1',
      }),
    );

    await client.sendReply('thread/with spaces', body);

    expect(fetchSpy.mock.calls[0]?.[0]).toEqual(
      new URL('http://localhost/api/threads/thread%2Fwith%20spaces/reply'),
    );
    expectMutatingJsonRequest(fetchSpy.mock.calls[0]?.[1], 'POST', body);
  });

  it('sends CSRF header and credentials for createDraft', async () => {
    const body = draftRequest();
    const fetchSpy = vi.spyOn(globalThis, 'fetch').mockResolvedValueOnce(
      jsonResponse(201, {
        draft_id: 'draft-1',
        updated_at: '2026-05-23T13:00:00Z',
      }),
    );

    await client.createDraft(body);

    expect(fetchSpy.mock.calls[0]?.[0]).toEqual(
      new URL('http://localhost/api/drafts'),
    );
    expectMutatingJsonRequest(fetchSpy.mock.calls[0]?.[1], 'POST', body);
  });

  it('sends CSRF header and credentials for updateDraft', async () => {
    const body = draftRequest({ subject: 'Updated subject' });
    const fetchSpy = vi.spyOn(globalThis, 'fetch').mockResolvedValueOnce(
      jsonResponse(200, {
        draft_id: 'draft-1',
        updated_at: '2026-05-23T13:00:00Z',
      }),
    );

    await client.updateDraft('draft/with spaces', body);

    expect(fetchSpy.mock.calls[0]?.[0]).toEqual(
      new URL('http://localhost/api/drafts/draft%2Fwith%20spaces'),
    );
    expectMutatingJsonRequest(fetchSpy.mock.calls[0]?.[1], 'PATCH', body);
  });
});

describe('HailApiClient non-composer mutating requests', () => {
  it('sends CSRF header and JSON body for setup admin', async () => {
    const body = {
      email: 'admin@example.org',
      password: 'correct horse battery staple',
      display_name: 'Admin',
      domain: 'example.org',
      bootstrap_token: 'operator-bootstrap-token',
    };
    const fetchSpy = vi.spyOn(globalThis, 'fetch').mockResolvedValueOnce(
      jsonResponse(201, {
        user: {
          id: 1,
          email: 'admin@example.org',
          display_name: 'Admin',
          is_admin: true,
        },
      }),
    );

    await client.createSetupAdmin(body);

    expect(fetchSpy.mock.calls[0]?.[0]).toEqual(
      new URL('http://localhost/api/setup/admin'),
    );
    expectMutatingJsonRequest(fetchSpy.mock.calls[0]?.[1], 'POST', body);
  });

  it('sends CSRF header and JSON body for screener decisions', async () => {
    const body: ScreenerDecisionRequest = {
      sender: 'lists@example.org',
      decision: 'approve',
      classify_as: 'feed',
      apply_to_history: true,
    };
    const fetchSpy = vi.spyOn(globalThis, 'fetch').mockResolvedValueOnce(
      jsonResponse(200, {
        sender: 'lists@example.org',
        decision: 'approve',
        classify_as: 'feed',
      }),
    );

    await client.decideScreener(body);

    expect(fetchSpy.mock.calls[0]?.[0]).toEqual(
      new URL('http://localhost/api/screener/decisions'),
    );
    expectMutatingJsonRequest(fetchSpy.mock.calls[0]?.[1], 'POST', body);
  });

  it('sends CSRF header and JSON body for undoing denied screener senders', async () => {
    const fetchSpy = vi
      .spyOn(globalThis, 'fetch')
      .mockResolvedValueOnce(
        jsonResponse(200, { status: 'approved', classify_as: 'papertrail' }),
      );

    await client.undoDeny('blocked+tag@example.org / team', {
      classify_as: 'papertrail',
    });

    expect(fetchSpy.mock.calls[0]?.[0]).toEqual(
      new URL(
        'http://localhost/api/screener/blocked%2Btag%40example.org%20%2F%20team/undo-deny',
      ),
    );
    expectMutatingJsonRequest(fetchSpy.mock.calls[0]?.[1], 'POST', {
      classify_as: 'papertrail',
    });
  });

  it('sends CSRF header and JSON body for admin mutating requests', async () => {
    const createBody = {
      email: 'agent@example.org',
      password: 'correct horse battery staple',
      display_name: 'Agent',
    };
    const resetBody = { password: 'new correct horse battery staple' };
    const domainBody = { domain: 'example.net' };
    const userEnvelope = {
      user: {
        id: 42,
        email: 'agent@example.org',
        display_name: 'Agent',
        is_admin: false,
      },
    };
    const fetchSpy = vi
      .spyOn(globalThis, 'fetch')
      .mockResolvedValueOnce(jsonResponse(201, userEnvelope))
      .mockResolvedValueOnce(jsonResponse(200, userEnvelope))
      .mockResolvedValueOnce(emptyResponse(204))
      .mockResolvedValueOnce(jsonResponse(201, { domain: 'example.net' }))
      .mockResolvedValueOnce(emptyResponse(204));

    await client.createAdminUser(createBody);
    await client.resetAdminUserPassword(42, resetBody);
    await client.deleteAdminUser(42);
    await client.addAdminDomain(domainBody);
    await client.deleteAdminDomain('example.net');

    expect(fetchSpy.mock.calls[0]?.[0]).toEqual(
      new URL('http://localhost/api/admin/users'),
    );
    expectMutatingJsonRequest(fetchSpy.mock.calls[0]?.[1], 'POST', createBody);
    expect(fetchSpy.mock.calls[1]?.[0]).toEqual(
      new URL('http://localhost/api/admin/users/42/reset-password'),
    );
    expectMutatingJsonRequest(fetchSpy.mock.calls[1]?.[1], 'POST', resetBody);
    expect(fetchSpy.mock.calls[2]?.[0]).toEqual(
      new URL('http://localhost/api/admin/users/42'),
    );
    expectMutatingNoBodyRequest(fetchSpy.mock.calls[2]?.[1], 'DELETE');
    expect(fetchSpy.mock.calls[3]?.[0]).toEqual(
      new URL('http://localhost/api/admin/domains'),
    );
    expectMutatingJsonRequest(fetchSpy.mock.calls[3]?.[1], 'POST', domainBody);
    expect(fetchSpy.mock.calls[4]?.[0]).toEqual(
      new URL('http://localhost/api/admin/domains/example.net'),
    );
    expectMutatingNoBodyRequest(fetchSpy.mock.calls[4]?.[1], 'DELETE');
  });

  it('sends CSRF header for login and logout', async () => {
    const loginBody = {
      email: 'reader@example.org',
      password: 'correct horse battery staple',
    };
    const fetchSpy = vi
      .spyOn(globalThis, 'fetch')
      .mockResolvedValueOnce(
        jsonResponse(200, {
          user: {
            id: 1,
            email: 'reader@example.org',
            display_name: 'Reader',
            is_admin: false,
          },
        }),
      )
      .mockResolvedValueOnce(emptyResponse(204));

    await client.login(loginBody);
    await client.logout();

    expect(fetchSpy.mock.calls[0]?.[0]).toEqual(
      new URL('http://localhost/api/auth/login'),
    );
    expectMutatingJsonRequest(fetchSpy.mock.calls[0]?.[1], 'POST', loginBody);
    expect(fetchSpy.mock.calls[1]?.[0]).toEqual(
      new URL('http://localhost/api/auth/logout'),
    );
    expectMutatingNoBodyRequest(fetchSpy.mock.calls[1]?.[1], 'POST');
  });

  it('sends CSRF header and JSON body for thread classification', async () => {
    const fetchSpy = vi
      .spyOn(globalThis, 'fetch')
      .mockResolvedValueOnce(jsonResponse(200, { undo: null }));

    await client.classifyThread('thread/with spaces', 'papertrail');

    expect(fetchSpy.mock.calls[0]?.[0]).toEqual(
      new URL('http://localhost/api/threads/thread%2Fwith%20spaces/classify'),
    );
    expectMutatingJsonRequest(fetchSpy.mock.calls[0]?.[1], 'POST', {
      to: 'papertrail',
    });
  });

  it('sends CSRF header without JSON content type for scheduled-send cancel', async () => {
    const fetchSpy = vi.spyOn(globalThis, 'fetch').mockResolvedValueOnce(
      jsonResponse(200, {
        id: 42,
        draft_email_id: 'draft-1',
        send_at: '2026-05-23T13:00:00Z',
        status: 'cancelled',
        created_at: '2026-05-23T12:00:00Z',
      }),
    );

    await client.cancelScheduledSend(42);

    expect(fetchSpy.mock.calls[0]?.[0]).toEqual(
      new URL('http://localhost/api/scheduled-sends/42'),
    );
    expectMutatingNoBodyRequest(fetchSpy.mock.calls[0]?.[1], 'DELETE');
  });

  it('sends CSRF header without JSON content type for bodyless thread verbs', async () => {
    const fetchSpy = vi
      .spyOn(globalThis, 'fetch')
      .mockResolvedValueOnce(jsonResponse(200, { undo: null }));

    await client.archiveThread('thread/with spaces');

    expect(fetchSpy.mock.calls[0]?.[0]).toEqual(
      new URL('http://localhost/api/threads/thread%2Fwith%20spaces/archive'),
    );
    expectMutatingNoBodyRequest(fetchSpy.mock.calls[0]?.[1], 'POST');
  });

  it('sends CSRF header without JSON content type for stack thread verbs', async () => {
    const fetchSpy = vi
      .spyOn(globalThis, 'fetch')
      .mockResolvedValueOnce(jsonResponse(200, { undo: null }))
      .mockResolvedValueOnce(jsonResponse(200, { undo: null }))
      .mockResolvedValueOnce(jsonResponse(200, { undo: null }))
      .mockResolvedValueOnce(jsonResponse(200, { status: 'restored' }))
      .mockResolvedValueOnce(jsonResponse(200, { status: 'destroyed' }))
      .mockResolvedValueOnce(jsonResponse(200, { status: 'undone' }));

    await client.setAsideThread('thread/with spaces');
    await client.replyLaterThread('thread/with spaces');
    await client.trashThread('thread/with spaces');
    await client.restoreThread('thread/with spaces');
    await client.destroyThread('thread/with spaces');
    await client.undo('undo/with spaces');

    expect(fetchSpy.mock.calls[0]?.[0]).toEqual(
      new URL('http://localhost/api/threads/thread%2Fwith%20spaces/set-aside'),
    );
    expectMutatingNoBodyRequest(fetchSpy.mock.calls[0]?.[1], 'POST');
    expect(fetchSpy.mock.calls[1]?.[0]).toEqual(
      new URL('http://localhost/api/threads/thread%2Fwith%20spaces/reply-later'),
    );
    expectMutatingNoBodyRequest(fetchSpy.mock.calls[1]?.[1], 'POST');
    expect(fetchSpy.mock.calls[2]?.[0]).toEqual(
      new URL('http://localhost/api/threads/thread%2Fwith%20spaces/trash'),
    );
    expectMutatingNoBodyRequest(fetchSpy.mock.calls[2]?.[1], 'POST');
    expect(fetchSpy.mock.calls[3]?.[0]).toEqual(
      new URL('http://localhost/api/threads/thread%2Fwith%20spaces/restore'),
    );
    expectMutatingNoBodyRequest(fetchSpy.mock.calls[3]?.[1], 'POST');
    expect(fetchSpy.mock.calls[4]?.[0]).toEqual(
      new URL('http://localhost/api/threads/thread%2Fwith%20spaces/destroy'),
    );
    expectMutatingNoBodyRequest(fetchSpy.mock.calls[4]?.[1], 'DELETE');
    expect(fetchSpy.mock.calls[5]?.[0]).toEqual(
      new URL('http://localhost/api/undo/undo%2Fwith%20spaces'),
    );
    expectMutatingNoBodyRequest(fetchSpy.mock.calls[5]?.[1], 'POST');
  });

  it('sends CSRF header and JSON body for thread note and mark updates', async () => {
    const noteBody: CreateThreadNoteRequest = {
      email_id: 'email-1',
      body: 'follow up after the meeting',
    };
    const fetchSpy = vi
      .spyOn(globalThis, 'fetch')
      .mockResolvedValueOnce(
        jsonResponse(201, {
          id: 7,
          email_id: 'email-1',
          body: 'follow up after the meeting',
          created_at: '2026-05-23T13:00:00Z',
        }),
      )
      .mockResolvedValueOnce(emptyResponse(204))
      .mockResolvedValueOnce(emptyResponse(204));

    await client.createThreadNote('thread/with spaces', noteBody);
    await client.deleteThreadNote('thread/with spaces', 7);
    await client.markThread('thread/with spaces', true);

    expect(fetchSpy.mock.calls[0]?.[0]).toEqual(
      new URL('http://localhost/api/threads/thread%2Fwith%20spaces/notes'),
    );
    expectMutatingJsonRequest(fetchSpy.mock.calls[0]?.[1], 'POST', noteBody);
    expect(fetchSpy.mock.calls[1]?.[0]).toEqual(
      new URL('http://localhost/api/threads/thread%2Fwith%20spaces/notes/7'),
    );
    expectMutatingNoBodyRequest(fetchSpy.mock.calls[1]?.[1], 'DELETE');
    expect(fetchSpy.mock.calls[2]?.[0]).toEqual(
      new URL('http://localhost/api/threads/thread%2Fwith%20spaces/mark'),
    );
    expectMutatingJsonRequest(fetchSpy.mock.calls[2]?.[1], 'POST', { read: true });
  });

  it('requires markThread to return the documented 204 status', async () => {
    vi.spyOn(globalThis, 'fetch').mockResolvedValueOnce(
      jsonResponse(200, { undo: null }),
    );

    await expectHailApiError(client.markThread('thread-1', false), {
      status: 200,
      body: { undo: null },
    });
  });

  it('sends CSRF header and JSON body for contact note updates', async () => {
    const body: PutContactNoteRequest = { markdown: 'met at !!con' };
    const fetchSpy = vi.spyOn(globalThis, 'fetch').mockResolvedValueOnce(
      jsonResponse(200, {
        markdown: 'met at !!con',
        updated_at: '2026-05-23T13:00:00Z',
      }),
    );

    await client.putContactNote('person+tag@example.org / team', body);

    expect(fetchSpy.mock.calls[0]?.[0]).toEqual(
      new URL(
        'http://localhost/api/contacts/person%2Btag%40example.org%20%2F%20team/note',
      ),
    );
    expectMutatingJsonRequest(fetchSpy.mock.calls[0]?.[1], 'PUT', body);
  });

  it('sends CSRF header without JSON content type for contact note deletes', async () => {
    const fetchSpy = vi
      .spyOn(globalThis, 'fetch')
      .mockResolvedValueOnce(emptyResponse(204));

    await client.deleteContactNote('person+tag@example.org / team');

    expect(fetchSpy.mock.calls[0]?.[0]).toEqual(
      new URL(
        'http://localhost/api/contacts/person%2Btag%40example.org%20%2F%20team/note',
      ),
    );
    expectMutatingNoBodyRequest(fetchSpy.mock.calls[0]?.[1], 'DELETE');
  });

  it('sends CSRF header and JSON body for bubble up scheduling', async () => {
    const body: BubbleUpRequest = { at: '2026-05-23T13:00:00Z' };
    const fetchSpy = vi.spyOn(globalThis, 'fetch').mockResolvedValueOnce(
      jsonResponse(201, {
        bubble_id: 1,
        surface_at: '2026-05-23T13:00:00Z',
      }),
    );

    await client.bubbleUpThread('thread/with spaces', body);

    expect(fetchSpy.mock.calls[0]?.[0]).toEqual(
      new URL('http://localhost/api/threads/thread%2Fwith%20spaces/bubble-up'),
    );
    expectMutatingJsonRequest(fetchSpy.mock.calls[0]?.[1], 'POST', body);
  });

  it('sends CSRF header without JSON content type for bubble up cancellation', async () => {
    const fetchSpy = vi
      .spyOn(globalThis, 'fetch')
      .mockResolvedValueOnce(jsonResponse(200, { status: 'cancelled' }));

    await client.cancelBubbleUp('thread/with spaces');

    expect(fetchSpy.mock.calls[0]?.[0]).toEqual(
      new URL('http://localhost/api/threads/thread%2Fwith%20spaces/bubble-up'),
    );
    expectMutatingNoBodyRequest(fetchSpy.mock.calls[0]?.[1], 'DELETE');
  });

  it('sends CSRF header without JSON content type for draft deletes', async () => {
    const fetchSpy = vi
      .spyOn(globalThis, 'fetch')
      .mockResolvedValueOnce(emptyResponse(204));

    await client.deleteDraft('draft/with spaces');

    expect(fetchSpy.mock.calls[0]?.[0]).toEqual(
      new URL('http://localhost/api/drafts/draft%2Fwith%20spaces'),
    );
    expectMutatingNoBodyRequest(fetchSpy.mock.calls[0]?.[1], 'DELETE');
  });
});

async function expectHailApiError(
  promise: Promise<unknown>,
  expected: { status: number; body: unknown },
) {
  await expect(promise).rejects.toBeInstanceOf(HailApiError);
  await expect(promise).rejects.toMatchObject(expected);
}

function composeRequest(overrides: Partial<ComposeRequest> = {}): ComposeRequest {
  return {
    to: ['bob@example.org'],
    subject: 'Hello',
    body_markdown: 'Body',
    ...overrides,
  };
}

function replyRequest(overrides: Partial<ReplyRequest> = {}): ReplyRequest {
  return {
    body_markdown: 'Reply body',
    ...overrides,
  };
}

function draftRequest(overrides: Partial<DraftRequest> = {}): DraftRequest {
  return {
    to: ['bob@example.org'],
    subject: 'Draft subject',
    body_markdown: 'Draft body',
    ...overrides,
  };
}

function expectMutatingJsonRequest(
  init: RequestInit | undefined,
  method: string,
  body: unknown,
) {
  expect(init?.method).toBe(method);
  expect(init?.credentials).toBe('include');
  expect(init?.body).toBe(JSON.stringify(body));

  const headers = expectHeaders(init);
  expect(headers.get('X-Hail-Request')).toBe('1');
  expect(headers.get('content-type')).toBe('application/json');
}

function expectMutatingNoBodyRequest(
  init: RequestInit | undefined,
  method: string,
) {
  expect(init?.method).toBe(method);
  expect(init?.credentials).toBe('include');
  expect(init?.body).toBeUndefined();

  const headers = expectHeaders(init);
  expect(headers.get('X-Hail-Request')).toBe('1');
  expect(headers.get('accept')).toBe('application/json');
  expect(headers.has('content-type')).toBe(false);
}

function expectGetRequest(init: RequestInit | undefined) {
  expect(init?.method).toBe('GET');
  expect(init?.credentials).toBe('include');
  expect(init?.body).toBeUndefined();

  const headers = expectHeaders(init);
  expect(headers.get('accept')).toBe('application/json');
  expect(headers.has('X-Hail-Request')).toBe(false);
  expect(headers.has('content-type')).toBe(false);
}

function expectHeaders(init: RequestInit | undefined): Headers {
  expect(init?.headers).toBeInstanceOf(Headers);
  return init?.headers as Headers;
}

function threadResponse() {
  return {
    thread_id: 'thread-1',
    messages: [],
    participants: [],
    subject: 'Thread subject',
  };
}
