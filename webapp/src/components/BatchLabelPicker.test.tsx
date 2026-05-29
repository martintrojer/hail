import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { QueryClientProvider } from '@tanstack/react-query';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type {
  BatchAssignLabelRequest,
  LabelItemResponse,
  LabelListResponse,
  LabelResponse,
} from '../api/client';
import { createTestQueryClient, TestHailApiClient } from '../test-utils';
import { BatchLabelPicker } from './BatchLabelPicker';

const labels: LabelResponse[] = [
  {
    id: 21,
    name: 'Work/Receipts',
    leaf_name: 'Receipts',
    path_segments: ['Work', 'Receipts'],
    source: 'manual',
    color: null,
    thread_count: 4,
  },
  {
    id: 22,
    name: 'Personal',
    leaf_name: 'Personal',
    path_segments: ['Personal'],
    source: 'manual',
    color: null,
    thread_count: 1,
  },
];

class BatchLabelPickerTestClient extends TestHailApiClient {
  readonly assignLabelToThreadsCalls: BatchAssignLabelRequest[] = [];

  override async listLabels(): Promise<LabelListResponse> {
    return { labels };
  }

  override async assignLabelToThreads(
    request: BatchAssignLabelRequest,
  ): Promise<LabelItemResponse> {
    this.assignLabelToThreadsCalls.push(request);
    return {
      label:
        labels.find((label) => label.id === request.label_id) ?? {
          id: 99,
          name: request.label_name ?? 'Created',
          leaf_name: request.label_name?.split('/').at(-1) ?? 'Created',
          path_segments: request.label_name?.split('/') ?? ['Created'],
          source: 'manual',
          color: null,
          thread_count: request.thread_ids.length,
        },
    };
  }
}

afterEach(() => {
  cleanup();
});

function renderPicker(client = new BatchLabelPickerTestClient(), onAssigned = vi.fn()) {
  render(
    <QueryClientProvider client={createTestQueryClient()}>
      <BatchLabelPicker
        client={client}
        count={2}
        threadIds={['thread-one', 'thread-two']}
        onAssigned={onAssigned}
      />
    </QueryClientProvider>,
  );

  return { client, onAssigned };
}

async function openPicker() {
  const trigger = screen.getByRole('button', { name: 'Label' });
  fireEvent.pointerDown(trigger, { button: 0, ctrlKey: false, pointerType: 'mouse' });
  fireEvent.click(trigger);
  return screen.findByPlaceholderText('Search or create label…');
}

describe('BatchLabelPicker keyboard behavior', () => {
  it('assigns an existing label with ArrowDown and Enter from the command input', async () => {
    const { client, onAssigned } = renderPicker();
    const input = await openPicker();

    await screen.findByRole('option', { name: 'Assign label Work/Receipts' });
    input.focus();
    fireEvent.keyDown(input, { key: 'ArrowDown', code: 'ArrowDown' });
    fireEvent.keyDown(input, { key: 'Enter', code: 'Enter' });

    await waitFor(() =>
      expect(client.assignLabelToThreadsCalls).toEqual([
        { thread_ids: ['thread-one', 'thread-two'], label_id: 21 },
      ]),
    );
    expect(onAssigned).toHaveBeenCalledTimes(1);
    await waitFor(() => expect(screen.queryByText('Assign label')).not.toBeInTheDocument());
  });

  it('creates and assigns a typed label with Enter on the create option', async () => {
    const { client, onAssigned } = renderPicker();
    const input = await openPicker();

    fireEvent.change(input, { target: { value: 'Projects/Hail' } });
    expect(
      await screen.findByRole('option', { name: 'Create and assign “Projects/Hail”' }),
    ).toBeInTheDocument();
    input.focus();
    fireEvent.keyDown(input, { key: 'Enter', code: 'Enter' });

    await waitFor(() =>
      expect(client.assignLabelToThreadsCalls).toEqual([
        { thread_ids: ['thread-one', 'thread-two'], label_name: 'Projects/Hail' },
      ]),
    );
    expect(onAssigned).toHaveBeenCalledTimes(1);
  });
});
