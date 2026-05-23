import { afterEach, describe, expect, it, vi } from 'vitest';
import { HailApiClient, type ComposeRequest } from './client';

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

function composeRequest(overrides: Partial<ComposeRequest> = {}): ComposeRequest {
  return {
    to: ['bob@example.org'],
    subject: 'Hello',
    body_markdown: 'Body',
    ...overrides,
  };
}
