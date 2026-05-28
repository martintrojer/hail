import { RouterProvider } from '@tanstack/react-router';
import { cleanup, fireEvent, screen, waitFor } from '@testing-library/react';
import type { ReactNode } from 'react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { ApiClientProvider } from '../api/ApiClientProvider';
import type {
  WorkflowRule,
  WorkflowRuleListResponse,
  WorkflowRulePayload,
  WorkflowRuleResponse,
} from '../api/client';
import { router } from '../router';
import {
  createTestQueryClient,
  renderWithQueryClient,
  seedMe,
  TestHailApiClient,
} from '../test-utils';
import { WorkflowsPage } from './WorkflowsPage';

if (!HTMLElement.prototype.hasPointerCapture) {
  HTMLElement.prototype.hasPointerCapture = () => false;
}
if (!HTMLElement.prototype.setPointerCapture) {
  HTMLElement.prototype.setPointerCapture = () => undefined;
}
if (!HTMLElement.prototype.scrollIntoView) {
  HTMLElement.prototype.scrollIntoView = () => undefined;
}

class WorkflowsTestClient extends TestHailApiClient {
  readonly createCalls: WorkflowRulePayload[] = [];
  readonly updateCalls: Array<{ id: number; body: WorkflowRulePayload }> = [];
  readonly deleteCalls: number[] = [];

  constructor(private rules: WorkflowRule[]) {
    super();
  }

  override async listWorkflows(): Promise<WorkflowRuleListResponse> {
    return { rules: this.rules };
  }

  override async createWorkflow(
    body: WorkflowRulePayload,
  ): Promise<WorkflowRuleResponse> {
    this.createCalls.push(body);
    const rule = workflowRule({
      id: 99,
      name: body.name,
      enabled: body.enabled ?? true,
      conditions: body.conditions,
      action: body.action,
    });
    this.rules = [rule, ...this.rules];
    return { rule };
  }

  override async updateWorkflow(
    id: number,
    body: WorkflowRulePayload,
  ): Promise<WorkflowRuleResponse> {
    this.updateCalls.push({ id, body });
    const rule = workflowRule({
      id,
      name: body.name,
      enabled: body.enabled ?? true,
      conditions: body.conditions,
      action: body.action,
    });
    this.rules = this.rules.map((item) => (item.id === id ? rule : item));
    return { rule };
  }

  override async deleteWorkflow(id: number): Promise<void> {
    this.deleteCalls.push(id);
    this.rules = this.rules.filter((rule) => rule.id !== id);
  }
}

function workflowRule(overrides: Partial<WorkflowRule> = {}): WorkflowRule {
  return {
    id: 7,
    name: 'Receipts to Paper Trail',
    enabled: true,
    conditions: [{ field: 'from', op: 'contains', value: 'billing@' }],
    action: { classify_as: 'papertrail', add_label: 'Receipts', auto_reply: null },
    created_at: '2026-05-26T12:00:00Z',
    updated_at: '2026-05-26T12:30:00Z',
    ...overrides,
  };
}

let currentTestBody: ReactNode = null;
let restoreWorkflowsRoute: (() => void) | null = null;

function TestBody() {
  return currentTestBody;
}

function installTestRouteComponent() {
  const matchRoute = router.routesByPath['/workflows'];
  const previousComponent = matchRoute.options.component;
  const previousBeforeLoad = matchRoute.options.beforeLoad;
  matchRoute.options.component = TestBody;
  matchRoute.options.beforeLoad = undefined;
  restoreWorkflowsRoute = () => {
    matchRoute.options.component = previousComponent;
    matchRoute.options.beforeLoad = previousBeforeLoad;
  };
}

function renderWorkflows(client: WorkflowsTestClient) {
  const queryClient = createTestQueryClient();
  seedMe(queryClient);
  currentTestBody = <WorkflowsPage client={client} />;
  installTestRouteComponent();
  window.history.pushState({}, '', '/workflows');
  renderWithQueryClient(
    <ApiClientProvider client={client}>
      <RouterProvider router={router} />
    </ApiClientProvider>,
    queryClient,
  );
}

afterEach(() => {
  currentTestBody = null;
  restoreWorkflowsRoute?.();
  restoreWorkflowsRoute = null;
  window.history.pushState({}, '', '/');
  cleanup();
  vi.restoreAllMocks();
});

describe('WorkflowsPage', () => {
  it('lists workflow rules and creates a rule from the builder', async () => {
    const client = new WorkflowsTestClient([workflowRule()]);
    renderWorkflows(client);

    expect(await screen.findByText('Receipts to Paper Trail')).toBeInTheDocument();
    expect(screen.getByText(/If From contains “billing@”/)).toBeInTheDocument();
    expect(screen.getByText(/Then route to Paper Trail/)).toBeInTheDocument();

    fireEvent.change(screen.getByLabelText('Name'), {
      target: { value: 'VIP to Imbox' },
    });
    fireEvent.change(screen.getByLabelText('Condition 1 value'), {
      target: { value: 'vip@example.com' },
    });
    fireEvent.pointerDown(screen.getByLabelText('Route to'), {
      button: 0,
      ctrlKey: false,
      pointerType: 'mouse',
    });
    fireEvent.click(await screen.findByRole('option', { name: 'Imbox' }));
    fireEvent.click(screen.getByRole('button', { name: 'Create workflow' }));

    await waitFor(() => expect(client.createCalls).toHaveLength(1));
    expect(client.createCalls[0]).toMatchObject({
      name: 'VIP to Imbox',
      enabled: true,
      conditions: [{ field: 'from', op: 'contains', value: 'vip@example.com' }],
      action: { classify_as: 'imbox', add_label: null, auto_reply: null },
    });
    expect(await screen.findByText('Workflow created.')).toBeInTheDocument();
    expect(await screen.findByText('VIP to Imbox')).toBeInTheDocument();
  });

  it('edits and deletes existing workflow rules', async () => {
    vi.spyOn(window, 'confirm').mockReturnValue(true);
    const client = new WorkflowsTestClient([workflowRule()]);
    renderWorkflows(client);

    fireEvent.click(await screen.findByRole('button', { name: 'Edit' }));
    fireEvent.change(screen.getByLabelText('Name'), {
      target: { value: 'Receipts off' },
    });
    fireEvent.click(screen.getByLabelText('Enabled'));
    fireEvent.change(screen.getByLabelText('Add label'), {
      target: { value: 'Finance' },
    });
    fireEvent.click(screen.getByRole('button', { name: 'Save workflow' }));

    await waitFor(() => expect(client.updateCalls).toHaveLength(1));
    expect(client.updateCalls[0]).toMatchObject({
      id: 7,
      body: { name: 'Receipts off', enabled: false },
    });
    expect(await screen.findByText('Workflow saved.')).toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: 'Delete' }));
    await waitFor(() => expect(client.deleteCalls).toEqual([7]));
    expect(await screen.findByText('Workflow deleted.')).toBeInTheDocument();
    expect(await screen.findByText('No workflows yet')).toBeInTheDocument();
  });
});
