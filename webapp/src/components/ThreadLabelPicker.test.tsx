import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { QueryClientProvider } from '@tanstack/react-query';
import { afterEach, describe, expect, it } from 'vitest';
import type {
  AssignLabelNameRequest,
  LabelItemResponse,
  LabelListResponse,
  LabelResponse,
} from '../api/client';
import { createTestQueryClient, TestHailApiClient } from '../test-utils';
import { ThreadLabelPicker } from './ThreadLabelPicker';

const labels: LabelResponse[] = [
  {
    id: 11,
    name: 'Projects/Hail',
    leaf_name: 'Hail',
    path_segments: ['Projects', 'Hail'],
    source: 'manual',
    color: null,
    thread_count: 2,
  },
  {
    id: 12,
    name: 'Personal',
    leaf_name: 'Personal',
    path_segments: ['Personal'],
    source: 'manual',
    color: null,
    thread_count: 1,
  },
];

class ThreadLabelPickerTestClient extends TestHailApiClient {
  readonly assignLabelCalls: Array<{ threadId: string; labelId: number }> = [];
  readonly assignLabelNameCalls: Array<{ threadId: string; labelName: string }> = [];

  override async listLabels(): Promise<LabelListResponse> {
    return { labels };
  }

  override async assignLabelToThread(
    threadId: string,
    labelId: number,
  ): Promise<LabelItemResponse> {
    this.assignLabelCalls.push({ threadId, labelId });
    return { label: labels.find((label) => label.id === labelId) ?? labels[0] };
  }

  override async assignLabelNameToThread(
    threadId: string,
    request: AssignLabelNameRequest,
  ): Promise<LabelItemResponse> {
    this.assignLabelNameCalls.push({ threadId, labelName: request.label_name });
    return {
      label: {
        id: 99,
        name: request.label_name,
        leaf_name: request.label_name.split('/').at(-1) ?? request.label_name,
        path_segments: request.label_name.split('/'),
        source: 'manual',
        color: null,
        thread_count: 1,
      },
    };
  }
}

afterEach(() => {
  cleanup();
});

function renderPicker(client = new ThreadLabelPickerTestClient()) {
  render(
    <QueryClientProvider client={createTestQueryClient()}>
      <ThreadLabelPicker threadId="thread-1" assignedLabels={[]} client={client} />
    </QueryClientProvider>,
  );

  return client;
}

async function openPicker() {
  const trigger = screen.getByRole('button', { name: 'Manage thread labels' });
  fireEvent.pointerDown(trigger, { button: 0, ctrlKey: false, pointerType: 'mouse' });
  fireEvent.click(trigger);
  return screen.findByPlaceholderText('Search or create label…');
}

describe('ThreadLabelPicker keyboard behavior', () => {
  it('assigns an existing label with ArrowDown and Enter from the command input', async () => {
    const client = renderPicker();
    const input = await openPicker();

    await screen.findByRole('option', { name: 'Add label Projects/Hail' });
    input.focus();
    fireEvent.keyDown(input, { key: 'ArrowDown', code: 'ArrowDown' });
    fireEvent.keyDown(input, { key: 'Enter', code: 'Enter' });

    await waitFor(() =>
      expect(client.assignLabelCalls).toEqual([{ threadId: 'thread-1', labelId: 11 }]),
    );
  });

  it('creates and assigns a typed label with Enter on the create option', async () => {
    const client = renderPicker();
    const input = await openPicker();

    fireEvent.change(input, { target: { value: 'Family/Kids' } });
    expect(await screen.findByRole('option', { name: 'Create “Family/Kids”' })).toBeInTheDocument();
    input.focus();
    fireEvent.keyDown(input, { key: 'Enter', code: 'Enter' });

    await waitFor(() =>
      expect(client.assignLabelNameCalls).toEqual([
        { threadId: 'thread-1', labelName: 'Family/Kids' },
      ]),
    );
  });
});
