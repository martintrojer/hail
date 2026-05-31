import { RouterProvider } from '@tanstack/react-router';
import {
  cleanup,
  fireEvent,
  screen,
  waitFor,
  within,
} from '@testing-library/react';
import type { ReactNode } from 'react';
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest';

const closeComposerMock = vi.hoisted(() => vi.fn());

vi.mock('../hooks/useGoBack', () => ({
  useGoBack: () => closeComposerMock,
}));
import type {
  ComposeRequest,
  ComposeResponse,
  DraftRequest,
  DraftResponse,
  ReplyRequest,
  ThreadViewResponse,
} from '../api/client';
import { HailApiError } from '../api/client';
import { AuthProvider } from '../auth/AuthProvider';
import { UndoToastProvider } from '../components/UndoToastProvider';
import { router } from '../router';
import {
  createTestQueryClient,
  renderWithQueryClient,
  seedMe,
  TestHailApiClient,
} from '../test-utils';
import { ComposerPage } from './ComposerPage';

class ComposerPageTestClient extends TestHailApiClient {
  readonly sendComposeCalls: ComposeRequest[] = [];
  readonly sendReplyCalls: Array<{ threadId: string; body: ReplyRequest }> = [];
  readonly createDraftCalls: DraftRequest[] = [];
  readonly updateDraftCalls: Array<{ draftId: string; body: DraftRequest }> =
    [];
  getDraftCalls: string[] = [];
  getThreadCalls: string[] = [];
  getDraftError: unknown;
  threadResponse: ThreadViewResponse = {
    thread_id: 'thread-123',
    subject: 'Launch plan',
    participants: [],
    notes: [],
    labels: [],
    messages: [
      {
        email_id: 'email-1',
        from: [{ email: 'alice@example.com', name: 'Alice' }],
        to: [{ email: 'composer@example.com', name: 'Composer' }],
        html: '<p>Can you review this?</p>',
        html_with_remote_images: '<p>Can you review this?</p>',
        reply_quote_html:
          '<p>On 2026-05-25T12:30:00+00:00, Alice wrote:</p><blockquote><p>Can you review this?</p></blockquote>',
        preview: 'Can you review this?\nThanks!',
        received_at: '2026-05-25T12:30:00Z',
        blocked_trackers: [],
      },
    ],
  };
  sendComposeError: unknown;
  sendReplyError: unknown;
  createDraftError: unknown;
  updateDraftError: unknown;
  draftResponse = {
    draft_id: 'draft-existing',
    to: ['alice@example.com', 'bob@example.com'],
    cc: ['carol@example.com'],
    bcc: ['dave@example.com'],
    subject: 'Saved draft subject',
    body_html:
      '<p>Saved draft body.</p><blockquote><p>Earlier context.</p></blockquote>',
    body_markdown: 'Saved draft body. Earlier context.',
  };

  constructor() {
    super({
      user: {
        id: 1,
        email: 'composer@example.com',
        display_name: 'Composer',
        is_admin: false,
      },
    });
  }

  override async getThread(threadId: string): Promise<ThreadViewResponse> {
    this.getThreadCalls.push(threadId);
    return this.threadResponse;
  }

  override async getDraft(draftId: string) {
    this.getDraftCalls.push(draftId);
    if (this.getDraftError) throw this.getDraftError;
    return this.draftResponse as Awaited<
      ReturnType<TestHailApiClient['getDraft']>
    >;
  }

  override async sendCompose(body: ComposeRequest): Promise<ComposeResponse> {
    this.sendComposeCalls.push(body);
    if (this.sendComposeError) throw this.sendComposeError;
    if (body.send_at) {
      return {
        status: 'pending',
        scheduled_send_id: 1,
        draft_email_id: 'draft-1',
      };
    }
    return {
      status: 'sent',
      email_id: 'email-1',
      submission_id: 'submission-1',
    };
  }

  override async sendReply(
    threadId: string,
    body: ReplyRequest,
  ): Promise<ComposeResponse> {
    this.sendReplyCalls.push({ threadId, body });
    if (this.sendReplyError) throw this.sendReplyError;
    if (body.send_at) {
      return {
        status: 'pending',
        scheduled_send_id: 2,
        draft_email_id: 'reply-draft-1',
      };
    }
    return {
      status: 'sent',
      email_id: 'reply-email-1',
      submission_id: 'reply-submission-1',
    };
  }

  override async createDraft(body: DraftRequest): Promise<DraftResponse> {
    this.createDraftCalls.push(body);
    if (this.createDraftError) throw this.createDraftError;
    return {
      draft_id: 'draft-1',
      updated_at: '2026-05-23T12:00:00Z',
    };
  }

  override async updateDraft(
    draftId: string,
    body: DraftRequest,
  ): Promise<DraftResponse> {
    this.updateDraftCalls.push({ draftId, body });
    if (this.updateDraftError) throw this.updateDraftError;
    return {
      draft_id: draftId,
      updated_at: '2026-05-23T12:05:00Z',
    };
  }
}

let currentTestBody: ReactNode = null;
let restoreComposeRoute: (() => void) | null = null;

declare global {
  interface Window {
    __HAIL_TEST_EDITOR_UPDATES__?: WeakMap<HTMLElement, (html: string) => void>;
    __HAIL_TEST_EDITORS__?: WeakMap<
      HTMLElement,
      import('@tiptap/react').Editor
    >;
  }
}

beforeAll(() => {
  Element.prototype.getClientRects = function getClientRects() {
    const rect = this.getBoundingClientRect();
    return {
      length: 1,
      item: (index: number) => (index === 0 ? rect : null),
      0: rect,
      [Symbol.iterator]: function* iterator() {
        yield rect;
      },
    } as DOMRectList;
  };
  document.elementFromPoint = () => document.body;
  Range.prototype.getClientRects = function getRangeClientRects() {
    const rect = document.body.getBoundingClientRect();
    return {
      length: 1,
      item: (index: number) => (index === 0 ? rect : null),
      0: rect,
      [Symbol.iterator]: function* iterator() {
        yield rect;
      },
    } as DOMRectList;
  };
  Range.prototype.getBoundingClientRect = () =>
    document.body.getBoundingClientRect();
});

afterEach(() => {
  vi.useRealTimers();
  closeComposerMock.mockReset();
  currentTestBody = null;
  restoreComposeRoute?.();
  restoreComposeRoute = null;
  window.history.pushState({}, '', '/');
  cleanup();
});

function TestBody() {
  return currentTestBody;
}

function installTestRouteComponent() {
  const matchRoute = router.routesByPath['/compose'];
  const previousComponent = matchRoute.options.component;
  const previousBeforeLoad = matchRoute.options.beforeLoad;
  matchRoute.options.component = TestBody;
  matchRoute.options.beforeLoad = undefined;
  restoreComposeRoute = () => {
    matchRoute.options.component = previousComponent;
    matchRoute.options.beforeLoad = previousBeforeLoad;
  };
}

interface RenderComposerOptions {
  client?: ComposerPageTestClient;
  replyToThreadId?: string;
  initialTo?: string[];
  initialSubject?: string;
  replyAll?: boolean;
  draftId?: string;
  inReplyToEmailId?: string;
  locationState?: Record<string, unknown>;
}

function renderComposer({
  client = new ComposerPageTestClient(),
  replyToThreadId,
  initialTo,
  initialSubject,
  replyAll,
  draftId,
  inReplyToEmailId,
  locationState = {},
}: RenderComposerOptions = {}) {
  const queryClient = createTestQueryClient();

  seedMe(queryClient, client.testUser);

  currentTestBody = (
    <AuthProvider>
      <UndoToastProvider>
        <ComposerPage
          client={client}
          replyToThreadId={replyToThreadId}
          replyAll={replyAll}
          draftId={draftId}
          inReplyToEmailId={inReplyToEmailId}
          initialTo={initialTo}
          initialSubject={initialSubject}
        />
      </UndoToastProvider>
    </AuthProvider>
  );
  installTestRouteComponent();
  window.history.pushState(locationState, '', '/compose');

  renderWithQueryClient(<RouterProvider router={router} />, queryClient);

  return client;
}

function setEditorHtmlForTest(editor: HTMLElement, html: string) {
  const update = window.__HAIL_TEST_EDITOR_UPDATES__?.get(editor);
  expect(update).toBeDefined();
  update?.(html);
}

function testEditorForElement(editor: HTMLElement) {
  const tiptapEditor = window.__HAIL_TEST_EDITORS__?.get(editor);
  expect(tiptapEditor).toBeDefined();
  return tiptapEditor!;
}

async function setEditorText(text: string) {
  const editor = await screen.findByLabelText('Body');
  setEditorHtmlForTest(editor, text ? `<p>${text}</p>` : '<p></p>');
  fireEvent.input(editor, {
    target: { innerHTML: text ? `<p>${text}</p>` : '<p></p>' },
  });
}

function getEditorHtml() {
  return screen.findByLabelText('Body').then((editor) => editor.innerHTML);
}

async function openLinkPopover() {
  const trigger = await screen.findByRole('button', { name: 'Link' });
  fireEvent.pointerDown(trigger, {
    button: 0,
    ctrlKey: false,
    pointerType: 'mouse',
  });
  fireEvent.click(trigger);

  const input = await screen.findByLabelText('Link URL');
  const content = input.closest('[data-slot="popover-content"]');
  expect(content).not.toBeNull();

  return {
    content: content as HTMLElement,
    input,
    trigger,
  };
}

function applyLinkFromPopover(content: HTMLElement) {
  fireEvent.click(within(content).getByRole('button', { name: 'Apply' }));
}

async function getEditorText() {
  return (await screen.findByLabelText('Body')).textContent ?? '';
}

async function fillSendableFields() {
  fireEvent.change(await screen.findByLabelText('To'), {
    target: { value: 'alice@example.com; bob@example.com' },
  });
  fireEvent.change(screen.getByLabelText('Cc'), {
    target: { value: 'carol@example.com' },
  });
  fireEvent.change(screen.getByLabelText('Bcc'), {
    target: { value: 'dave@example.com, erin@example.com' },
  });
  fireEvent.change(screen.getByLabelText('Subject'), {
    target: { value: 'Quarterly report' },
  });
  await setEditorText('Report attached.');
  await screen.findByText('Draft not saved yet');
}

async function fillReplyBody() {
  await setEditorText('Reply from the composer.');
}

function selectAttachment() {
  const file = new File(['hello'], 'report.pdf', { type: 'application/pdf' });
  fireEvent.change(screen.getByLabelText('Attachments'), {
    target: { files: [file] },
  });
}

function dateTimeLocalValue(date: Date) {
  const offsetMs = date.getTimezoneOffset() * 60_000;
  return new Date(date.getTime() - offsetMs).toISOString().slice(0, 16);
}

function apiError(status: number, body: unknown) {
  return new HailApiError(
    status,
    body,
    new Response(JSON.stringify(body), { status }),
  );
}

describe('ComposerPage', () => {
  it('uses the centralized AppShell composer container instead of a route max-width wrapper', async () => {
    renderComposer();

    expect(
      await screen.findByRole('button', { name: 'Cancel' }),
    ).toBeInTheDocument();
    const content = screen.getByTestId('app-shell-content');
    expect(content).toHaveAttribute('data-hail-content-layout', 'composer');
    expect(content).toHaveClass(
      'max-w-3xl',
      'lg:max-w-4xl',
      'xl:max-w-5xl',
      'min-w-0',
    );
  });

  it('blocks sending while attachments are selected', async () => {
    const client = renderComposer();
    await fillSendableFields();
    selectAttachment();

    expect(screen.getByRole('button', { name: 'Send now' })).toBeDisabled();
    expect(
      screen.getByText(
        /Attachments are not supported for sending or saving yet\./,
      ),
    ).toBeInTheDocument();
    fireEvent.submit(
      screen.getByRole('button', { name: 'Send now' }).closest('form')!,
    );

    expect(
      await screen.findByText(
        /Attachments are selected, but sending and saving attachments is not supported yet\./,
      ),
    ).toBeInTheDocument();
    expect(client.sendComposeCalls).toEqual([]);
  });

  it('blocks send later while attachments are selected', async () => {
    const client = renderComposer();
    await fillSendableFields();
    fireEvent.change(screen.getByLabelText('Send later'), {
      target: {
        value: dateTimeLocalValue(new Date(Date.now() + 60 * 60 * 1000)),
      },
    });
    selectAttachment();

    expect(screen.getByRole('button', { name: 'Send later' })).toBeDisabled();
    expect(
      screen.getByText(
        /Attachments are not supported for sending or saving yet\./,
      ),
    ).toBeInTheDocument();
    expect(client.sendComposeCalls).toEqual([]);
  });

  it('blocks saving drafts while attachments are selected', async () => {
    const client = renderComposer();
    await fillSendableFields();
    selectAttachment();

    expect(screen.getByRole('button', { name: 'Save draft' })).toBeDisabled();
    expect(
      screen.getByText(
        /Attachments are not supported for sending or saving yet\./,
      ),
    ).toBeInTheDocument();
    expect(client.createDraftCalls).toEqual([]);
  });

  it('renders the rich-text toolbar and updates editor HTML for formatting toggles', async () => {
    renderComposer();
    await setEditorText('Hello rich text');
    const editor = await screen.findByLabelText('Body');

    fireEvent.click(screen.getByRole('button', { name: 'Bold' }));
    setEditorHtmlForTest(editor, '<p><strong>Bold words</strong></p>');
    expect(await getEditorHtml()).toContain('<strong>Bold words</strong>');

    fireEvent.click(screen.getByRole('button', { name: 'Bold' }));
    fireEvent.click(screen.getByRole('button', { name: 'Italic' }));
    setEditorHtmlForTest(editor, '<p><em>Italic words</em></p>');
    expect(await getEditorHtml()).toContain('<em>Italic words</em>');

    fireEvent.click(screen.getByRole('button', { name: 'Italic' }));
    fireEvent.click(screen.getByRole('button', { name: 'Bullet list' }));
    setEditorHtmlForTest(editor, '<ul><li><p>List item</p></li></ul>');
    expect(await getEditorHtml()).toContain('<ul>');
    expect(await getEditorText()).toContain('List item');
  });

  it('sends the editor HTML without a legacy markdown body', async () => {
    const client = renderComposer();
    fireEvent.change(await screen.findByLabelText('To'), {
      target: { value: 'alice@example.com' },
    });
    fireEvent.change(screen.getByLabelText('Subject'), {
      target: { value: 'Rich message' },
    });
    setEditorHtmlForTest(
      await screen.findByLabelText('Body'),
      '<p>Hello <strong>Alice</strong></p><ul><li><p>First</p></li></ul>',
    );
    await waitFor(() =>
      expect(
        screen.getByRole('button', { name: 'Send now' }),
      ).not.toBeDisabled(),
    );

    fireEvent.click(screen.getByRole('button', { name: 'Send now' }));

    await waitFor(() => expect(client.sendComposeCalls).toHaveLength(1));
    expect(client.sendComposeCalls[0]).toEqual({
      to: ['alice@example.com'],
      cc: [],
      bcc: [],
      subject: 'Rich message',
      body_html:
        '<p>Hello <strong>Alice</strong></p><ul><li><p>First</p></li></ul><p></p>',
      attachments: [],
    });
    expect(client.sendComposeCalls[0]).not.toHaveProperty('body_markdown');
  });

  it('pre-sanitizes pasted HTML before inserting it', async () => {
    renderComposer();
    const editor = await screen.findByLabelText('Body');

    fireEvent.paste(editor, {
      clipboardData: {
        getData: (type: string) => {
          if (type === 'text/html') {
            return '<p onclick="alert(1)">Safe <strong>text</strong><script>alert(1)</script><iframe src="https://evil.example"></iframe><a href="javascript:alert(1)">bad</a><a href="https://example.com/path">good</a></p>';
          }
          return '';
        },
      },
    });

    await waitFor(() =>
      expect(getEditorHtml()).resolves.toContain('<strong>text</strong>'),
    );
    const html = await getEditorHtml();
    expect(html).toContain('Safe');
    expect(html).toContain('bad');
    expect(html).toContain('href="https://example.com/path"');
    expect(html).toContain('rel="noopener noreferrer"');
    expect(html).toContain('target="_blank"');
    expect(html).not.toContain('script');
    expect(html).not.toContain('iframe');
    expect(html).not.toContain('onclick');
    expect(html).not.toContain('javascript:');
  });

  it('auto-links URLs when pasting plain text', async () => {
    renderComposer();
    const editor = await screen.findByLabelText('Body');

    fireEvent.paste(editor, {
      clipboardData: {
        getData: (type: string) =>
          type === 'text/plain' ? 'Read https://example.com/docs.' : '',
      },
    });

    await waitFor(() => expect(getEditorHtml()).resolves.toContain('<a'));
    const html = await getEditorHtml();
    expect(html).toContain('href="https://example.com/docs"');
    expect(html).toContain('target="_blank"');
    expect(html).toContain('rel="noopener noreferrer"');
    expect(html).toContain('docs</a>.');
  });

  it('applies and removes links from the link popover', async () => {
    renderComposer();
    const editor = await screen.findByLabelText('Body');
    const tiptapEditor = testEditorForElement(editor);
    tiptapEditor.commands.setContent('<p>Example link</p>');
    tiptapEditor.commands.setTextSelection({ from: 1, to: 8 });

    const invalidPopover = await openLinkPopover();
    fireEvent.change(invalidPopover.input, {
      target: { value: 'javascript:alert(1)' },
    });
    applyLinkFromPopover(invalidPopover.content);

    expect(
      await within(invalidPopover.content).findByText(
        'Enter an http, https, or mailto URL.',
      ),
    ).toBeInTheDocument();
    expect(await getEditorHtml()).not.toContain('javascript:');

    const validPopover = await openLinkPopover();
    fireEvent.change(validPopover.input, {
      target: { value: 'https://example.com' },
    });
    applyLinkFromPopover(validPopover.content);

    await waitFor(() => expect(getEditorHtml()).resolves.toContain('<a'));
    let html = await getEditorHtml();
    expect(html).toContain('href="https://example.com/"');
    expect(html).toContain('target="_blank"');
    expect(html).toContain('rel="noopener noreferrer"');

    tiptapEditor.commands.setTextSelection({ from: 1, to: 8 });
    const removePopover = await openLinkPopover();
    fireEvent.click(
      within(removePopover.content).getByRole('button', { name: 'Remove' }),
    );

    await waitFor(() => expect(getEditorHtml()).resolves.not.toContain('<a'));
    html = await getEditorHtml();
    expect(html).not.toContain('href=');
  });

  it('supports rich-text keyboard shortcuts for bold, italic, and lists', async () => {
    renderComposer();
    const editor = await screen.findByLabelText('Body');
    const tiptapEditor = testEditorForElement(editor);
    tiptapEditor.commands.setContent('<p>Bold words</p>');
    tiptapEditor.commands.setTextSelection({ from: 1, to: 11 });

    fireEvent.keyDown(editor, { key: 'b', ctrlKey: true });
    await waitFor(() =>
      expect(getEditorHtml()).resolves.toContain('<strong>Bold words</strong>'),
    );

    fireEvent.keyDown(editor, { key: 'i', ctrlKey: true });
    await waitFor(() =>
      expect(getEditorHtml()).resolves.toContain(
        '<strong><em>Bold words</em></strong>',
      ),
    );

    tiptapEditor.commands.setContent('<p>List item</p>');
    tiptapEditor.commands.setTextSelection(1);
    fireEvent.keyDown(editor, { key: '8', ctrlKey: true, shiftKey: true });
    await waitFor(() => expect(getEditorHtml()).resolves.toContain('<ul>'));

    fireEvent.keyDown(editor, { key: '7', ctrlKey: true, shiftKey: true });
    await waitFor(() => expect(getEditorHtml()).resolves.toContain('<ol>'));
  });

  it('lets Enter leave empty list and quote blocks', async () => {
    renderComposer();
    const editor = await screen.findByLabelText('Body');
    const tiptapEditor = testEditorForElement(editor);

    tiptapEditor.commands.setContent('<ul><li><p></p></li></ul>');
    tiptapEditor.commands.setTextSelection(3);
    fireEvent.keyDown(editor, { key: 'Enter' });
    await waitFor(() => expect(getEditorHtml()).resolves.not.toContain('<ul>'));

    tiptapEditor.commands.setContent('<blockquote><p></p></blockquote>');
    tiptapEditor.commands.setTextSelection(1);
    fireEvent.keyDown(editor, { key: 'Enter' });
    await waitFor(() =>
      expect(getEditorHtml()).resolves.not.toContain('<blockquote>'),
    );
  });

  it.each([
    ['Ctrl', { ctrlKey: true }],
    ['Meta', { metaKey: true }],
  ] as const)(
    'sends the current editor HTML on %s+Enter from editor focus',
    async (_label, modifier) => {
      const client = renderComposer();
      fireEvent.change(await screen.findByLabelText('To'), {
        target: { value: 'alice@example.com' },
      });
      fireEvent.change(screen.getByLabelText('Subject'), {
        target: { value: 'Shortcut message' },
      });
      const editor = await screen.findByLabelText('Body');
      setEditorHtmlForTest(
        editor,
        '<p>Hello <strong>Alice</strong></p><blockquote><p>Previous context</p></blockquote>',
      );
      editor.focus();
      await waitFor(() =>
        expect(
          screen.getByRole('button', { name: 'Send now' }),
        ).not.toBeDisabled(),
      );
      const currentBodyHtml =
        '<p>Hello <strong>Alice</strong></p><blockquote><p>Previous context</p></blockquote><p></p>';

      fireEvent.keyDown(window, {
        key: 'Enter',
        code: 'Enter',
        target: editor,
        ...modifier,
      });

      await waitFor(() => expect(client.sendComposeCalls).toHaveLength(1));
      expect(client.sendComposeCalls[0]).toEqual({
        to: ['alice@example.com'],
        cc: [],
        bcc: [],
        subject: 'Shortcut message',
        body_html: currentBodyHtml,
        attachments: [],
      });
    },
  );

  it('closes the composer on Escape from editor focus', async () => {
    renderComposer();
    await setEditorText('Draft text before closing.');
    const editor = await screen.findByLabelText('Body');
    editor.focus();

    fireEvent.keyDown(window, {
      key: 'Escape',
      code: 'Escape',
      target: editor,
    });

    expect(closeComposerMock).toHaveBeenCalledTimes(1);
  });

  it('does not send on Cmd/Ctrl+Enter from editor focus when the form cannot submit', async () => {
    const client = renderComposer();
    fireEvent.change(await screen.findByLabelText('Subject'), {
      target: { value: 'Missing recipient' },
    });
    await setEditorText('This body still needs a recipient.');
    const editor = await screen.findByLabelText('Body');
    editor.focus();
    expect(screen.getByRole('button', { name: 'Send now' })).toBeDisabled();

    fireEvent.keyDown(window, {
      key: 'Enter',
      code: 'Enter',
      target: editor,
      ctrlKey: true,
    });
    fireEvent.keyDown(window, {
      key: 'Enter',
      code: 'Enter',
      target: editor,
      metaKey: true,
    });

    expect(client.sendComposeCalls).toEqual([]);
  });

  it('still sends when no attachments are selected', async () => {
    const client = renderComposer();
    await fillSendableFields();

    expect(screen.getByRole('button', { name: 'Send now' })).not.toBeDisabled();
    fireEvent.submit(
      screen.getByRole('button', { name: 'Send now' }).closest('form')!,
    );

    await waitFor(() => expect(client.sendComposeCalls).toHaveLength(1));
    expect(client.sendComposeCalls[0]).toMatchObject({
      to: ['alice@example.com', 'bob@example.com'],
      cc: ['carol@example.com'],
      bcc: ['dave@example.com', 'erin@example.com'],
      subject: 'Quarterly report',
      body_html: '<p>Report attached.</p>',
      attachments: [],
    });
    expect(await screen.findByText('Sent.')).toBeInTheDocument();
  });

  it('Sending from fresh compose route navigates to /imbox after success', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const client = renderComposer();
    await fillSendableFields();

    fireEvent.click(screen.getByRole('button', { name: 'Send now' }));

    await waitFor(() => expect(client.sendComposeCalls).toHaveLength(1));
    expect(await screen.findByText('Sent.')).toBeInTheDocument();
    expect(window.location.pathname).toBe('/compose');

    await vi.advanceTimersByTimeAsync(600);

    await waitFor(() => expect(window.location.pathname).toBe('/imbox'));
  });

  it('Replying from a thread navigates back to that thread', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const client = renderComposer({
      replyToThreadId: 'thread-123',
      locationState: { from: '/thread/thread-123' },
    });
    await screen.findByText('Reply to thread');
    await fillReplyBody();
    await waitFor(() =>
      expect(
        screen.getByRole('button', { name: 'Send now' }),
      ).not.toBeDisabled(),
    );

    fireEvent.click(screen.getByRole('button', { name: 'Send now' }));

    await waitFor(() => expect(client.sendReplyCalls).toHaveLength(1));
    expect(await screen.findByText('Sent.')).toBeInTheDocument();

    await vi.advanceTimersByTimeAsync(600);

    await waitFor(() =>
      expect(`${window.location.pathname}${window.location.search}`).toBe(
        '/thread/thread-123',
      ),
    );
  });

  it('Unsaved-changes prompt does not fire after successful send', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const confirmSpy = vi.spyOn(window, 'confirm');
    const client = renderComposer();
    await fillSendableFields();

    expect(screen.getByText('Draft not saved yet')).toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: 'Send now' }));

    await waitFor(() => expect(client.sendComposeCalls).toHaveLength(1));
    await screen.findByText('Sent.');
    expect(screen.queryByText('Draft not saved yet')).not.toBeInTheDocument();
    await vi.advanceTimersByTimeAsync(600);

    await waitFor(() => expect(window.location.pathname).toBe('/imbox'));
    expect(confirmSpy).not.toHaveBeenCalled();
  });

  it('rejects stale send-later datetimes before calling the API', async () => {
    const client = renderComposer();
    await fillSendableFields();
    fireEvent.change(screen.getByLabelText('Send later'), {
      target: { value: '2000-01-01T00:00' },
    });

    fireEvent.click(screen.getByRole('button', { name: 'Send later' }));

    expect(
      await screen.findByText('Choose a future send-later time.'),
    ).toBeInTheDocument();
    expect(client.sendComposeCalls).toHaveLength(0);
  });

  it('sends an ISO send_at when the datetime is future', async () => {
    const sendAtValue = dateTimeLocalValue(
      new Date(Date.now() + 60 * 60 * 1000),
    );
    const client = renderComposer();
    await fillSendableFields();
    fireEvent.change(screen.getByLabelText('Send later'), {
      target: { value: sendAtValue },
    });

    fireEvent.click(screen.getByRole('button', { name: 'Send later' }));

    await waitFor(() => expect(client.sendComposeCalls).toHaveLength(1));
    expect(client.sendComposeCalls[0]?.send_at).toBe(
      new Date(sendAtValue).toISOString(),
    );
    expect(
      await screen.findByText('Scheduled for later. Draft draft-1 is queued.'),
    ).toBeInTheDocument();
  });

  it('loads an existing draft and updates it instead of creating another draft', async () => {
    const client = new ComposerPageTestClient();
    renderComposer({ client, draftId: 'draft-existing' });

    expect(await screen.findByLabelText('To')).toHaveValue(
      'alice@example.com, bob@example.com',
    );
    expect(screen.getByLabelText('Cc')).toHaveValue('carol@example.com');
    expect(screen.getByLabelText('Bcc')).toHaveValue('dave@example.com');
    expect(screen.getByLabelText('Subject')).toHaveValue('Saved draft subject');
    expect(await getEditorText()).toBe('Saved draft body.Earlier context.');
    expect(await getEditorHtml()).toContain(
      '<blockquote><p>Earlier context.</p></blockquote>',
    );
    expect(client.getDraftCalls).toEqual(['draft-existing']);

    await setEditorText('Updated resumed draft body.');
    await waitFor(() =>
      expect(
        screen.getByRole('button', { name: 'Save draft' }),
      ).not.toBeDisabled(),
    );
    fireEvent.click(screen.getByRole('button', { name: 'Save draft' }));

    await waitFor(() => expect(client.updateDraftCalls).toHaveLength(1));
    expect(client.createDraftCalls).toEqual([]);
    expect(client.updateDraftCalls[0]).toEqual({
      draftId: 'draft-existing',
      body: {
        to: ['alice@example.com', 'bob@example.com'],
        cc: ['carol@example.com'],
        bcc: ['dave@example.com'],
        subject: 'Saved draft subject',
        body_html: '<p>Updated resumed draft body.</p>',
        body_markdown: 'Updated resumed draft body.',
        attachments: [],
      },
    });
  });

  it('loads legacy body_markdown when a draft has no body_html', async () => {
    const client = new ComposerPageTestClient();
    client.draftResponse = {
      ...client.draftResponse,
      body_html: undefined as unknown as string,
      body_markdown: 'Legacy markdown draft body.',
    };
    renderComposer({ client, draftId: 'draft-existing' });

    expect(await getEditorText()).toBe('Legacy markdown draft body.');
    await setEditorText('Migrated legacy body.');
    await waitFor(() =>
      expect(
        screen.getByRole('button', { name: 'Save draft' }),
      ).not.toBeDisabled(),
    );
    fireEvent.click(screen.getByRole('button', { name: 'Save draft' }));

    await waitFor(() => expect(client.updateDraftCalls).toHaveLength(1));
    expect(client.updateDraftCalls[0].body).toMatchObject({
      body_html: '<p>Migrated legacy body.</p>',
      body_markdown: 'Migrated legacy body.',
    });
  });

  it('creates a partial draft without send-required fields', async () => {
    const client = renderComposer();
    fireEvent.change(await screen.findByLabelText('Subject'), {
      target: { value: 'Unfinished thought' },
    });

    expect(screen.getByRole('button', { name: 'Send now' })).toBeDisabled();
    expect(
      screen.getByRole('button', { name: 'Save draft' }),
    ).not.toBeDisabled();
    fireEvent.click(screen.getByRole('button', { name: 'Save draft' }));

    await waitFor(() => expect(client.createDraftCalls).toHaveLength(1));
    expect(client.sendComposeCalls).toEqual([]);
    expect(client.createDraftCalls[0]).toEqual({
      to: [],
      cc: [],
      bcc: [],
      subject: 'Unfinished thought',
      body_html: '',
      body_markdown: '',
      attachments: [],
    });
  });

  it('keeps send validation strict when a partial draft is saveable', async () => {
    const client = renderComposer();
    await setEditorText('Needs a recipient and subject first.');
    await waitFor(() =>
      expect(screen.getByText('Draft not saved yet')).toBeInTheDocument(),
    );

    expect(screen.getByRole('button', { name: 'Send now' })).toBeDisabled();
    expect(screen.getByRole('button', { name: 'Send later' })).toBeDisabled();
    expect(
      screen.getByRole('button', { name: 'Save draft' }),
    ).not.toBeDisabled();
    fireEvent.click(screen.getByRole('button', { name: 'Save draft' }));

    await waitFor(() => expect(client.createDraftCalls).toHaveLength(1));
    expect(client.sendComposeCalls).toEqual([]);
    expect(client.createDraftCalls[0]).toMatchObject({
      to: [],
      subject: '',
      body_html: '<p>Needs a recipient and subject first.</p>',
      body_markdown: 'Needs a recipient and subject first.',
    });
  });

  it('creates a draft, then updates the same draft after more edits', async () => {
    const client = renderComposer();
    await fillSendableFields();

    fireEvent.click(screen.getByRole('button', { name: 'Save draft' }));

    await waitFor(() => expect(client.createDraftCalls).toHaveLength(1));
    expect(client.createDraftCalls[0]).toEqual({
      to: ['alice@example.com', 'bob@example.com'],
      cc: ['carol@example.com'],
      bcc: ['dave@example.com', 'erin@example.com'],
      subject: 'Quarterly report',
      body_html: '<p>Report attached.</p>',
      body_markdown: 'Report attached.',
      attachments: [],
    });
    expect(client.updateDraftCalls).toEqual([]);

    await setEditorText('Updated draft body.');
    await waitFor(() =>
      expect(
        screen.getByRole('button', { name: 'Save draft' }),
      ).not.toBeDisabled(),
    );
    fireEvent.click(screen.getByRole('button', { name: 'Save draft' }));

    await waitFor(() => expect(client.updateDraftCalls).toHaveLength(1));
    expect(client.createDraftCalls).toHaveLength(1);
    expect(client.updateDraftCalls[0]).toEqual({
      draftId: 'draft-1',
      body: {
        to: ['alice@example.com', 'bob@example.com'],
        cc: ['carol@example.com'],
        bcc: ['dave@example.com', 'erin@example.com'],
        subject: 'Quarterly report',
        body_html: '<p>Updated draft body.</p>',
        body_markdown: 'Updated draft body.',
        attachments: [],
      },
    });
  });

  it('autosaves dirty compose fields and then updates the created draft on the next interval', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const client = renderComposer();
    await fillSendableFields();

    expect(screen.getByText('Draft not saved yet')).toBeInTheDocument();

    await vi.advanceTimersByTimeAsync(5000);

    await waitFor(() => expect(client.createDraftCalls).toHaveLength(1));
    expect(client.createDraftCalls[0]).toEqual({
      to: ['alice@example.com', 'bob@example.com'],
      cc: ['carol@example.com'],
      bcc: ['dave@example.com', 'erin@example.com'],
      subject: 'Quarterly report',
      body_html: '<p>Report attached.</p>',
      body_markdown: 'Report attached.',
      attachments: [],
    });
    expect(await screen.findByText('Draft saved')).toBeInTheDocument();

    await setEditorText('Autosaved update.');

    await vi.advanceTimersByTimeAsync(5000);

    await waitFor(() => expect(client.updateDraftCalls).toHaveLength(1));
    expect(client.createDraftCalls).toHaveLength(1);
    expect(client.updateDraftCalls[0]).toEqual({
      draftId: 'draft-1',
      body: {
        to: ['alice@example.com', 'bob@example.com'],
        cc: ['carol@example.com'],
        bcc: ['dave@example.com', 'erin@example.com'],
        subject: 'Quarterly report',
        body_html: '<p>Autosaved update.</p>',
        body_markdown: 'Autosaved update.',
        attachments: [],
      },
    });
  });

  it('autosave snapshot changes when only body_html changes', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const client = new ComposerPageTestClient();
    client.draftResponse = {
      ...client.draftResponse,
      body_html: '<p>Same visible text</p>',
      body_markdown: 'Same visible text',
    };
    renderComposer({ client, draftId: 'draft-existing' });

    expect(await getEditorText()).toBe('Same visible text');
    setEditorHtmlForTest(
      await screen.findByLabelText('Body'),
      '<blockquote><p>Same visible text</p></blockquote>',
    );
    await waitFor(() =>
      expect(screen.getByText('Draft not saved yet')).toBeInTheDocument(),
    );

    await vi.advanceTimersByTimeAsync(5000);

    await waitFor(() => expect(client.updateDraftCalls).toHaveLength(1));
    expect(client.updateDraftCalls[0].draftId).toBe('draft-existing');
    expect(client.updateDraftCalls[0].body).toMatchObject({
      to: ['alice@example.com', 'bob@example.com'],
      cc: ['carol@example.com'],
      bcc: ['dave@example.com'],
      subject: 'Saved draft subject',
      body_html: '<blockquote><p>Same visible text</p></blockquote><p></p>',
      attachments: [],
    });
  });

  it('shows draft load errors and keeps compose fields available', async () => {
    const client = new ComposerPageTestClient();
    client.getDraftError = apiError(404, { error: 'not_found' });
    renderComposer({ client, draftId: 'missing-draft' });

    expect(await screen.findByLabelText('To')).toHaveValue('');
    expect(screen.getByLabelText('Subject')).toHaveValue('');
    expect(await getEditorText()).toBe('');
    expect(client.getDraftCalls).toEqual(['missing-draft']);
    expect(
      await screen.findByText('Draft could not be loaded. HTTP 404.'),
    ).toBeInTheDocument();
  });

  it('uses reply mode controls and sends through the reply API', async () => {
    const client = renderComposer({
      replyToThreadId: 'thread-123',
      initialTo: ['ignored@example.com'],
      initialSubject: 'Hidden subject',
    });
    await screen.findByText('Reply to thread');

    expect(await screen.findByLabelText('To')).toHaveValue('alice@example.com');
    expect(screen.getByLabelText('Subject')).toHaveValue('Re: Launch plan');
    expect(
      screen.queryByRole('button', { name: 'Save draft' }),
    ).not.toBeInTheDocument();

    fireEvent.change(screen.getByLabelText('Cc'), {
      target: { value: 'ignored-cc@example.com' },
    });
    await fillReplyBody();
    await waitFor(() =>
      expect(
        screen.getByRole('button', { name: 'Send now' }),
      ).not.toBeDisabled(),
    );
    fireEvent.click(screen.getByRole('button', { name: 'Send now' }));

    await waitFor(() => expect(client.sendReplyCalls).toHaveLength(1));
    expect(client.sendComposeCalls).toEqual([]);
    expect(client.sendReplyCalls[0]).toEqual({
      threadId: 'thread-123',
      body: {
        body_html: '<p>Reply from the composer.</p>',
        attachments: [],
        send_at: undefined,
      },
    });
    expect(await screen.findByText('Sent.')).toBeInTheDocument();
  });

  it('prefills reply-all recipients and quoted body while excluding the current user', async () => {
    const client = new ComposerPageTestClient();
    client.threadResponse = {
      thread_id: 'thread-456',
      subject: 'Re: Existing subject',
      participants: [],
      notes: [],
      messages: [
        {
          email_id: 'email-2',
          from: [{ email: 'bob@example.com', name: 'Bob Sender' }],
          to: [
            { email: 'composer@example.com', name: 'Composer' },
            { email: 'team@example.com', name: 'Team' },
            { email: 'bob@example.com', name: 'Bob Sender' },
          ],
          cc: [
            { email: 'carol@example.com', name: 'Carol' },
            { email: 'composer@example.com', name: 'Composer' },
          ],
          html: '<p>Line one</p>',
          html_with_remote_images: '<p>Line one</p>',
          reply_quote_html:
            '<p>On 2026-05-25T13:45:00+00:00, Bob Sender wrote:</p><blockquote><p>Line one</p></blockquote>',
          preview: 'Line one\nLine two',
          received_at: '2026-05-25T13:45:00Z',
          blocked_trackers: [],
        },
      ],
    } as unknown as ThreadViewResponse;

    renderComposer({ client, replyToThreadId: 'thread-456', replyAll: true });

    expect(await screen.findByLabelText('To')).toHaveValue(
      'bob@example.com, team@example.com',
    );
    expect(screen.getByLabelText('Cc')).toHaveValue('carol@example.com');
    expect(screen.getByLabelText('Subject')).toHaveValue(
      'Re: Existing subject',
    );
    const bodyText = await getEditorText();
    expect(bodyText).toContain('Bob Sender wrote:');
    expect(bodyText).toContain('Line one');
    expect(client.getThreadCalls).toEqual(['thread-456']);
  });

  it('prefills the quote from in_reply_to instead of the last thread message', async () => {
    const client = new ComposerPageTestClient();
    client.threadResponse = {
      thread_id: 'thread-789',
      subject: 'Specific message quote',
      participants: [],
      notes: [],
      labels: [],
      messages: [
        {
          email_id: 'email-1',
          from: [{ email: 'alice@example.com', name: 'Alice' }],
          to: [{ email: 'composer@example.com', name: 'Composer' }],
          html: '<p>First body</p>',
          html_with_remote_images: '<p>First body</p>',
          reply_quote_html:
            '<p>Alice wrote:</p><blockquote><p>First body</p></blockquote>',
          preview: 'First body',
          received_at: '2026-05-25T12:00:00Z',
          blocked_trackers: [],
        },
        {
          email_id: 'email-2',
          from: [{ email: 'bob@example.com', name: 'Bob' }],
          to: [{ email: 'composer@example.com', name: 'Composer' }],
          html: '<p>Specific email two body</p>',
          html_with_remote_images: '<p>Specific email two body</p>',
          reply_quote_html:
            '<p>Bob wrote:</p><blockquote><p>Specific email two body</p></blockquote>',
          preview: 'Specific email two body',
          received_at: '2026-05-25T13:00:00Z',
          blocked_trackers: [],
        },
        {
          email_id: 'email-4',
          from: [{ email: 'dana@example.com', name: 'Dana' }],
          to: [{ email: 'composer@example.com', name: 'Composer' }],
          html: '<p>Last message body</p>',
          html_with_remote_images: '<p>Last message body</p>',
          reply_quote_html:
            '<p>Dana wrote:</p><blockquote><p>Last message body</p></blockquote>',
          preview: 'Last message body',
          received_at: '2026-05-25T15:00:00Z',
          blocked_trackers: [],
        },
      ],
    } as unknown as ThreadViewResponse;

    renderComposer({
      client,
      replyToThreadId: 'thread-789',
      inReplyToEmailId: 'email-2',
    });

    const editorHtml = await getEditorHtml();
    expect(editorHtml).toContain(
      '<blockquote><p>Specific email two body</p></blockquote>',
    );
    expect(editorHtml).not.toContain('Last message body');
    expect(screen.getByLabelText('To')).toHaveValue('bob@example.com');
  });

  it('renders thread notes when replying', async () => {
    const client = new ComposerPageTestClient();
    client.threadResponse = {
      ...client.threadResponse,
      notes: [
        {
          id: 7,
          email_id: 'email-1',
          body: 'Mention the signed contract before sending.',
          created_at: '2026-05-20T10:15:00Z',
        },
      ],
    };

    renderComposer({ client, replyToThreadId: 'thread-123' });

    expect(await screen.findByText('Notes on this thread')).toBeInTheDocument();
    expect(
      screen.getByText('Mention the signed contract before sending.'),
    ).toBeInTheDocument();
    expect(screen.getByText(/You ·/)).toBeInTheDocument();
  });

  it('does not render thread notes for new compositions', async () => {
    renderComposer();

    expect(await screen.findByLabelText('Body')).toBeInTheDocument();
    expect(screen.queryByText('Notes on this thread')).not.toBeInTheDocument();
  });

  it('shows a loading state while fetching reply prefill data', async () => {
    renderComposer({ replyToThreadId: 'thread-123' });

    expect(screen.getByText('Loading reply details…')).toBeInTheDocument();
    expect(await getEditorHtml()).toContain(
      '<blockquote><p>Can you review this?</p></blockquote>',
    );
    expect((await screen.findByLabelText('Body')).textContent).toContain(
      'Can you review this?',
    );
  });

  it('shows send mutation error messages from API failures', async () => {
    const client = new ComposerPageTestClient();
    client.sendComposeError = apiError(400, { error: 'invalid_recipient' });
    renderComposer({ client });
    await fillSendableFields();

    fireEvent.click(screen.getByRole('button', { name: 'Send now' }));

    await waitFor(() => expect(client.sendComposeCalls).toHaveLength(1));
    expect(
      await screen.findByText('Check recipient addresses and try again.'),
    ).toBeInTheDocument();
  });

  it('shows draft mutation error messages from API failures', async () => {
    const client = new ComposerPageTestClient();
    client.createDraftError = apiError(400, { error: 'invalid_subject' });
    renderComposer({ client });
    await fillSendableFields();

    fireEvent.click(screen.getByRole('button', { name: 'Save draft' }));

    await waitFor(() => expect(client.createDraftCalls).toHaveLength(1));
    expect(
      await screen.findByText('Check the subject and try again.'),
    ).toBeInTheDocument();
  });

  it('keeps attachment details visible and blocks all mutations while files are selected', async () => {
    const client = renderComposer();
    await fillSendableFields();
    selectAttachment();

    const attachmentNotice = screen
      .getByText(/Attachments are not supported for sending or saving yet\./)
      .closest('div');
    expect(attachmentNotice).not.toBeNull();
    expect(
      within(attachmentNotice!).getByText(
        /report\.pdf · 5 B · application\/pdf/,
      ),
    ).toBeInTheDocument();

    expect(screen.getByRole('button', { name: 'Send now' })).toBeDisabled();
    expect(screen.getByRole('button', { name: 'Send later' })).toBeDisabled();
    expect(screen.getByRole('button', { name: 'Save draft' })).toBeDisabled();
    expect(client.sendComposeCalls).toEqual([]);
    expect(client.createDraftCalls).toEqual([]);
    expect(client.updateDraftCalls).toEqual([]);
    expect(client.sendReplyCalls).toEqual([]);
  });
});
