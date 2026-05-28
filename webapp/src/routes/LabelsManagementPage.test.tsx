import type { ReactNode } from 'react';
import { RouterProvider } from '@tanstack/react-router';
import { cleanup, fireEvent, screen, waitFor, within } from '@testing-library/react';
import { afterEach, describe, expect, it } from 'vitest';
import type {
  CreateLabelRequest,
  LabelItemResponse,
  LabelListResponse,
  RenameLabelRequest,
} from '../api/client';
import { ApiClientProvider } from '../api/ApiClientProvider';
import { AuthProvider } from '../auth/AuthProvider';
import { UndoToastProvider } from '../components/UndoToastProvider';
import {
  createTestQueryClient,
  renderWithQueryClient,
  seedMe,
  installTestRoute,
  TestHailApiClient,
} from '../test-utils';
import { router } from '../router';
import { LabelsManagementPage } from './LabelsManagementPage';

class LabelsManagementPageTestClient extends TestHailApiClient {
  labels: LabelListResponse = {
    labels: [
      {
        id: 3,
        name: 'Work/Receipts',
        leaf_name: 'Receipts',
        path_segments: ['Work', 'Receipts'],
        source: 'manual',
        color: null,
        thread_count: 2,
      },
      {
        id: 1,
        name: 'Family',
        leaf_name: 'Family',
        path_segments: ['Family'],
        source: 'manual',
        color: null,
        thread_count: 1,
      },
      {
        id: 2,
        name: 'Work',
        leaf_name: 'Work',
        path_segments: ['Work'],
        source: 'gmail',
        color: null,
        thread_count: 4,
      },
    ],
  };
  readonly createCalls: CreateLabelRequest[] = [];
  readonly renameCalls: Array<{ id: number; request: RenameLabelRequest }> = [];
  readonly deleteCalls: number[] = [];

  override async listLabels(): Promise<LabelListResponse> {
    return this.labels;
  }

  override async createLabel(request: CreateLabelRequest): Promise<LabelItemResponse> {
    this.createCalls.push(request);
    const label = {
      id: 4,
      name: request.name,
      leaf_name: request.name.split('/').at(-1) ?? request.name,
      path_segments: request.name.split('/'),
      source: 'manual' as const,
      color: request.color ?? null,
      thread_count: 0,
    };
    this.labels = { labels: [...this.labels.labels, label] };
    return { label };
  }

  override async renameLabel(id: number, request: RenameLabelRequest): Promise<LabelItemResponse> {
    this.renameCalls.push({ id, request });
    const existing = this.labels.labels.find((label) => label.id === id);
    const label = {
      ...(existing ?? this.labels.labels[0]),
      id,
      name: request.name,
      leaf_name: request.name.split('/').at(-1) ?? request.name,
      path_segments: request.name.split('/'),
    };
    this.labels = {
      labels: this.labels.labels.map((item) => (item.id === id ? label : item)),
    };
    return { label };
  }

  override async deleteLabel(id: number): Promise<void> {
    this.deleteCalls.push(id);
    this.labels = { labels: this.labels.labels.filter((label) => label.id !== id) };
  }
}

class CreateResponseOnlyLabelsClient extends LabelsManagementPageTestClient {
  private readonly afterCreateRefetch = new Promise<LabelListResponse>(() => {});
  private listCalls = 0;

  override async listLabels(): Promise<LabelListResponse> {
    this.listCalls += 1;
    if (this.listCalls > 1) {
      return this.afterCreateRefetch;
    }
    return this.labels;
  }

  override async createLabel(request: CreateLabelRequest): Promise<LabelItemResponse> {
    this.createCalls.push(request);
    return {
      label: {
        id: 4,
        name: request.name,
        leaf_name: request.name.split('/').at(-1) ?? request.name,
        path_segments: request.name.split('/'),
        source: 'manual',
        color: request.color ?? null,
        thread_count: 0,
      },
    };
  }
}

let currentTestBody: ReactNode = null;
let restoreLabelsRoute: (() => void) | null = null;

function TestBody() {
  return currentTestBody;
}

function renderLabelsPage(client = new LabelsManagementPageTestClient()) {
  const queryClient = createTestQueryClient();
  seedMe(queryClient);
  currentTestBody = (
    <ApiClientProvider client={client}>
      <AuthProvider>
        <UndoToastProvider>
          <LabelsManagementPage client={client} />
        </UndoToastProvider>
      </AuthProvider>
    </ApiClientProvider>
  );
  restoreLabelsRoute = installTestRoute(router, '/labels', {
    component: TestBody,
    beforeLoad: undefined,
  });
  window.history.pushState({}, '', '/labels');

  renderWithQueryClient(<RouterProvider router={router} />, queryClient);

  return client;
}

afterEach(() => {
  currentTestBody = null;
  restoreLabelsRoute?.();
  restoreLabelsRoute = null;
  window.history.pushState({}, '', '/');
  cleanup();
});

describe('LabelsManagementPage', () => {
  it('renders labels in deterministic tree-like order with full paths', async () => {
    renderLabelsPage();

    expect(await screen.findByRole('heading', { name: 'Labels' })).toBeInTheDocument();
    const rows = within(await screen.findByTestId('labels-list')).getAllByRole('listitem');

    expect(rows).toHaveLength(3);
    expect(rows[0]).toHaveTextContent('Family');
    expect(rows[1]).toHaveTextContent('Work');
    expect(rows[2]).toHaveTextContent('Work / Receipts');
    expect(within(rows[2]).getByText('Work/Receipts')).toBeInTheDocument();
    expect(within(rows[2]).getByText('2 threads')).toBeInTheDocument();
    expect(within(rows[2]).getByText('under Work')).toBeInTheDocument();
  });

  it('creates labels by full path', async () => {
    const client = renderLabelsPage();

    fireEvent.click(await screen.findByRole('button', { name: 'Create label' }));
    fireEvent.change(screen.getByLabelText('Label name or path'), {
      target: { value: 'Projects/Hail' },
    });
    fireEvent.click(screen.getByRole('button', { name: 'Create label' }));

    await waitFor(() => {
      expect(client.createCalls).toEqual([{ name: 'Projects/Hail' }]);
    });
    expect(await within(await screen.findByTestId('labels-list')).findByText('Projects / Hail')).toBeInTheDocument();
  });

  it('shows newly created labels from the mutation response before the labels refetch completes', async () => {
    const client = renderLabelsPage(new CreateResponseOnlyLabelsClient());

    fireEvent.click(await screen.findByRole('button', { name: 'Create label' }));
    fireEvent.change(screen.getByLabelText('Label name or path'), {
      target: { value: 'Projects/Hail' },
    });
    fireEvent.click(screen.getByRole('button', { name: 'Create label' }));

    await waitFor(() => {
      expect(client.createCalls).toEqual([{ name: 'Projects/Hail' }]);
    });
    expect(await within(await screen.findByTestId('labels-list')).findByText('Projects / Hail')).toBeInTheDocument();
  });

  it('renames labels', async () => {
    const client = renderLabelsPage();

    const receiptsRow = await within(await screen.findByTestId('labels-list')).findByText('Work / Receipts');
    fireEvent.click(within(receiptsRow.closest('li') as HTMLElement).getByRole('button', { name: 'Rename' }));
    const input = screen.getByLabelText('Label name or path');
    fireEvent.change(input, { target: { value: 'Work/Invoices' } });
    fireEvent.click(screen.getByRole('button', { name: 'Rename label' }));

    await waitFor(() => {
      expect(client.renameCalls).toEqual([{ id: 3, request: { name: 'Work/Invoices' } }]);
    });
    expect(await within(await screen.findByTestId('labels-list')).findByText('Work / Invoices')).toBeInTheDocument();
  });

  it('deletes labels only after warning confirmation', async () => {
    const client = renderLabelsPage();

    const labelsList = await screen.findByTestId('labels-list');
    const familyRow = within(labelsList).getAllByText('Family')[0];
    fireEvent.click(within(familyRow.closest('li') as HTMLElement).getByRole('button', { name: 'Delete Family' }));

    expect(screen.getByText(/removes it from every assigned thread/i)).toBeInTheDocument();
    expect(client.deleteCalls).toEqual([]);

    fireEvent.click(screen.getByRole('button', { name: 'Delete label' }));

    await waitFor(() => {
      expect(client.deleteCalls).toEqual([1]);
    });
    await waitFor(() => {
      expect(screen.queryByText('Family')).not.toBeInTheDocument();
    });
  });
});
