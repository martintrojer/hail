import { describe, expect, it, vi } from 'vitest';
import { QueryClient } from '@tanstack/react-query';
import {
  invalidateQueriesForEvent,
  parseHailEvent,
} from './events';
import { queryKeys } from './queryKeys';

describe('parseHailEvent', () => {
  it('accepts all client-supported event types and normalizes known payload fields', () => {
    expect(parseHailEvent('{"type":"imbox.new","thread_id":"t-imbox"}')).toEqual({
      type: 'imbox.new',
      threadId: 't-imbox',
    });
    expect(parseHailEvent('{"type":"feed.new","thread_id":"t-feed"}')).toEqual({
      type: 'feed.new',
      threadId: 't-feed',
    });
    expect(parseHailEvent('{"type":"papertrail.new","thread_id":"t-paper"}')).toEqual({
      type: 'papertrail.new',
      threadId: 't-paper',
    });
    expect(parseHailEvent('{"type":"thread.updated","thread_id":"t-updated"}')).toEqual({
      type: 'thread.updated',
      threadId: 't-updated',
    });
    expect(parseHailEvent('{"type":"thread.removed","thread_id":"t-removed"}')).toEqual({
      type: 'thread.removed',
      threadId: 't-removed',
    });
    expect(parseHailEvent('{"type":"bubble.fired","thread_id":"t-bubble"}')).toEqual({
      type: 'bubble.fired',
      threadId: 't-bubble',
    });
    expect(parseHailEvent('{"type":"send.completed","scheduled_send_id":42}')).toEqual({
      type: 'send.completed',
      scheduledSendId: 42,
    });
    expect(
      parseHailEvent(
        '{"type":"send.failed","scheduled_send_id":42,"error":"smtp rejected"}',
      ),
    ).toEqual({
      type: 'send.failed',
      scheduledSendId: 42,
      error: 'smtp rejected',
    });
    expect(parseHailEvent('{"type":"heartbeat","at":"2026-05-23T00:00:00Z"}')).toEqual({
      type: 'heartbeat',
      at: '2026-05-23T00:00:00Z',
    });
  });

  it('rejects malformed and unknown events', () => {
    expect(parseHailEvent(null)).toBeNull();
    expect(parseHailEvent('not json')).toBeNull();
    expect(parseHailEvent('{"type":"clips.new"}')).toBeNull();
  });
});

describe('invalidateQueriesForEvent', () => {
  it('invalidates the destination view and thread cache for new item events', () => {
    const { queryClient, invalidateSpy } = spiedQueryClient();

    invalidateQueriesForEvent(queryClient, { type: 'feed.new', threadId: 't-feed' });
    invalidateQueriesForEvent(queryClient, {
      type: 'papertrail.new',
      threadId: 't-paper',
    });

    expect(invalidateSpy).toHaveBeenCalledWith({ queryKey: queryKeys.view('feed') });
    expect(invalidateSpy).toHaveBeenCalledWith({ queryKey: queryKeys.thread('t-feed') });
    expect(invalidateSpy).toHaveBeenCalledWith({
      queryKey: queryKeys.view('papertrail'),
    });
    expect(invalidateSpy).toHaveBeenCalledWith({
      queryKey: queryKeys.thread('t-paper'),
    });
  });

  it('uses thread_id payloads to invalidate exact thread queries where practical', () => {
    const { queryClient, invalidateSpy } = spiedQueryClient();

    invalidateQueriesForEvent(queryClient, {
      type: 'thread.updated',
      threadId: 'thread-1',
    });
    invalidateQueriesForEvent(queryClient, {
      type: 'bubble.fired',
      threadId: 'thread-2',
    });

    expect(invalidateSpy).toHaveBeenCalledWith({
      queryKey: queryKeys.thread('thread-1'),
    });
    expect(invalidateSpy).toHaveBeenCalledWith({
      queryKey: queryKeys.thread('thread-2'),
    });
    expect(invalidateSpy).toHaveBeenCalledWith({ queryKey: queryKeys.views() });
    expect(invalidateSpy).not.toHaveBeenCalledWith({ queryKey: queryKeys.threads() });
  });

  it('falls back to broad thread invalidation when thread payloads are absent', () => {
    const { queryClient, invalidateSpy } = spiedQueryClient();

    invalidateQueriesForEvent(queryClient, { type: 'thread.updated' });

    expect(invalidateSpy).toHaveBeenCalledWith({ queryKey: queryKeys.threads() });
    expect(invalidateSpy).toHaveBeenCalledWith({ queryKey: queryKeys.views() });
  });

  it('removes exact thread caches for thread.removed and refreshes views', () => {
    const { queryClient, invalidateSpy, removeSpy } = spiedQueryClient();

    invalidateQueriesForEvent(queryClient, {
      type: 'thread.removed',
      threadId: 'removed-thread',
    });

    expect(removeSpy).toHaveBeenCalledWith({
      queryKey: queryKeys.thread('removed-thread'),
    });
    expect(invalidateSpy).toHaveBeenCalledWith({ queryKey: queryKeys.views() });
    expect(invalidateSpy).not.toHaveBeenCalledWith({ queryKey: queryKeys.threads() });
  });
});

function spiedQueryClient() {
  const queryClient = new QueryClient();
  const invalidateSpy = vi
    .spyOn(queryClient, 'invalidateQueries')
    .mockResolvedValue(undefined);
  const removeSpy = vi.spyOn(queryClient, 'removeQueries').mockReturnValue(undefined);

  return { queryClient, invalidateSpy, removeSpy };
}
