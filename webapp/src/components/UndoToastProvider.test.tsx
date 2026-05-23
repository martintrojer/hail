import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { act, type ReactNode } from 'react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { defaultApiClient } from '../api/query';
import { queryClient } from '../lib/queryClient';
import { UndoToastProvider, useUndoToast } from './UndoToastProvider';

function ToastControls() {
  const { showToast } = useUndoToast();

  return (
    <div>
      <button
        type="button"
        onClick={() =>
          showToast({
            message: 'Moved to Paper Trail.',
            undo: { id: 'undo-move-1', label: 'Undo move' },
            undoSuccessMessage: 'Move undone.',
            undoFailureMessage: 'Could not undo move.',
            durationMs: 10_000,
          })
        }
      >
        Show undoable toast
      </button>
      <button
        type="button"
        onClick={() =>
          showToast({
            message: 'Saved note.',
            durationMs: 10_000,
          })
        }
      >
        Show plain toast
      </button>
      <button
        type="button"
        onClick={() =>
          showToast({
            message: 'First toast.',
            durationMs: 1_000,
          })
        }
      >
        Show first timed toast
      </button>
      <button
        type="button"
        onClick={() =>
          showToast({
            message: 'Replacement toast.',
            durationMs: 5_000,
          })
        }
      >
        Show replacement toast
      </button>
    </div>
  );
}

function renderProvider(children: ReactNode = <ToastControls />) {
  return render(<UndoToastProvider>{children}</UndoToastProvider>);
}

afterEach(() => {
  vi.useRealTimers();
  vi.restoreAllMocks();
  queryClient.clear();
  cleanup();
});

describe('UndoToastProvider', () => {
  it('renders pushed toasts and dismisses them', () => {
    renderProvider();

    fireEvent.click(screen.getByRole('button', { name: 'Show undoable toast' }));

    expect(screen.getByRole('status')).toHaveTextContent('Moved to Paper Trail.');
    expect(screen.getByRole('button', { name: 'Undo move' })).toBeEnabled();

    fireEvent.click(screen.getByRole('button', { name: 'Dismiss notification' }));

    expect(screen.queryByRole('status')).not.toBeInTheDocument();
  });

  it('calls the undo API, invalidates hail queries, and renders success', async () => {
    const undo = vi.spyOn(defaultApiClient, 'undo').mockResolvedValue({
      id: 'undo-move-1',
      action: 'thread.classify',
    });
    const invalidateQueries = vi
      .spyOn(queryClient, 'invalidateQueries')
      .mockResolvedValue(undefined);

    renderProvider();
    fireEvent.click(screen.getByRole('button', { name: 'Show undoable toast' }));
    fireEvent.click(screen.getByRole('button', { name: 'Undo move' }));

    expect(screen.getByRole('button', { name: 'Undoing…' })).toBeDisabled();

    await waitFor(() => expect(undo).toHaveBeenCalledWith('undo-move-1'));
    expect(invalidateQueries).toHaveBeenCalledWith({ queryKey: ['hail'] });
    expect(await screen.findByRole('status')).toHaveTextContent('Move undone.');
    expect(screen.queryByRole('button', { name: 'Undo move' })).not.toBeInTheDocument();
  });

  it('renders failure and skips invalidation when undo fails', async () => {
    vi.spyOn(defaultApiClient, 'undo').mockRejectedValue(new Error('boom'));
    const invalidateQueries = vi.spyOn(queryClient, 'invalidateQueries');

    renderProvider();
    fireEvent.click(screen.getByRole('button', { name: 'Show undoable toast' }));
    fireEvent.click(screen.getByRole('button', { name: 'Undo move' }));

    expect(await screen.findByText('Could not undo move.')).toBeInTheDocument();
    expect(invalidateQueries).not.toHaveBeenCalled();
    expect(screen.queryByRole('button', { name: 'Undo move' })).not.toBeInTheDocument();
  });

  it('does not render an undo button for plain notifications', () => {
    renderProvider();

    fireEvent.click(screen.getByRole('button', { name: 'Show plain toast' }));

    expect(screen.getByRole('status')).toHaveTextContent('Saved note.');
    expect(screen.queryByRole('button', { name: 'Undo' })).not.toBeInTheDocument();
  });

  it('times out the current toast without letting stale timers clear replacements', async () => {
    vi.useFakeTimers();
    renderProvider();

    fireEvent.click(screen.getByRole('button', { name: 'Show first timed toast' }));
    expect(screen.getByRole('status')).toHaveTextContent('First toast.');

    await act(async () => {
      vi.advanceTimersByTime(500);
    });
    fireEvent.click(screen.getByRole('button', { name: 'Show replacement toast' }));
    expect(screen.getByRole('status')).toHaveTextContent('Replacement toast.');

    await act(async () => {
      vi.advanceTimersByTime(500);
    });
    expect(screen.getByRole('status')).toHaveTextContent('Replacement toast.');

    await act(async () => {
      vi.advanceTimersByTime(4_499);
    });
    expect(screen.getByRole('status')).toHaveTextContent('Replacement toast.');

    await act(async () => {
      vi.advanceTimersByTime(1);
    });
    expect(screen.queryByRole('status')).not.toBeInTheDocument();
  });
});
