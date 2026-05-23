import { afterEach, describe, expect, it, vi } from 'vitest';
import {
  HailApiClient,
  type ComposeRequest,
  type DraftRequest,
  type ReplyRequest,
} from './client';

const client = new HailApiClient({ baseUrl: 'http://localhost' });

function jsonResponse(status: number, body: unknown) {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

afterEach(() => {
  vi.restoreAllMocks();
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
  expect(init?.headers).toBeInstanceOf(Headers);

  const headers = init?.headers as Headers;
  expect(headers.get('X-Hail-Request')).toBe('1');
  expect(headers.get('content-type')).toBe('application/json');
}
