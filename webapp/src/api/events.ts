import { useEffect } from 'react';
import type { QueryClient } from '@tanstack/react-query';
import { queryKeys } from './queryKeys';

const INITIAL_RECONNECT_DELAY_MS = 1_000;
const MAX_RECONNECT_DELAY_MS = 30_000;

const eventTypes = [
  'imbox.new',
  'feed.new',
  'papertrail.new',
  'screener.pending',
  'thread.updated',
  'thread.removed',
  'bubble.fired',
  'send.completed',
  'send.failed',
  'heartbeat',
] as const;

export type HailEventType = (typeof eventTypes)[number];

export type HailEvent =
  | { type: 'heartbeat'; at?: string }
  | {
      type: 'imbox.new' | 'feed.new' | 'papertrail.new';
      threadId?: string;
    }
  | { type: 'screener.pending' }
  | { type: 'thread.updated' | 'thread.removed' | 'bubble.fired'; threadId?: string }
  | {
      type: 'send.completed' | 'send.failed';
      scheduledSendId?: number;
      error?: string;
    };

interface UseHailEventsOptions {
  enabled: boolean;
  queryClient: QueryClient;
}

export function useHailEvents({ enabled, queryClient }: UseHailEventsOptions) {
  useEffect(() => {
    if (!enabled || typeof window === 'undefined') {
      return undefined;
    }

    let active = true;
    let socket: WebSocket | null = null;
    let reconnectTimer: ReturnType<typeof window.setTimeout> | null = null;
    let attempt = 0;

    function clearReconnectTimer() {
      if (reconnectTimer !== null) {
        window.clearTimeout(reconnectTimer);
        reconnectTimer = null;
      }
    }

    function scheduleReconnect() {
      if (!active || reconnectTimer !== null) {
        return;
      }

      const baseDelay = Math.min(
        INITIAL_RECONNECT_DELAY_MS * 2 ** attempt,
        MAX_RECONNECT_DELAY_MS,
      );
      const jitter = Math.floor(Math.random() * 250);
      attempt += 1;
      reconnectTimer = window.setTimeout(() => {
        reconnectTimer = null;
        connect();
      }, baseDelay + jitter);
    }

    function connect() {
      if (!active) {
        return;
      }

      socket = new WebSocket(hailEventsUrl(window.location));

      socket.addEventListener('open', () => {
        attempt = 0;
      });

      socket.addEventListener('message', (message) => {
        const event = parseHailEvent(message.data);
        if (event === null) {
          return;
        }
        invalidateQueriesForEvent(queryClient, event);
      });

      socket.addEventListener('close', () => {
        socket = null;
        scheduleReconnect();
      });

      socket.addEventListener('error', () => {
        socket?.close();
      });
    }

    connect();

    return () => {
      active = false;
      clearReconnectTimer();
      socket?.close();
      socket = null;
    };
  }, [enabled, queryClient]);
}

export function hailEventsUrl(location: Location) {
  const url = new URL('/api/ws', location.href);
  url.protocol = location.protocol === 'https:' ? 'wss:' : 'ws:';
  return url;
}

export function parseHailEvent(data: unknown): HailEvent | null {
  if (typeof data !== 'string') {
    return null;
  }

  let parsed: unknown;
  try {
    parsed = JSON.parse(data);
  } catch {
    return null;
  }

  if (!isObject(parsed) || typeof parsed.type !== 'string') {
    return null;
  }

  if (!isHailEventType(parsed.type)) {
    return null;
  }

  if (parsed.type === 'heartbeat') {
    return typeof parsed.at === 'string'
      ? { type: parsed.type, at: parsed.at }
      : { type: parsed.type };
  }

  if (isNewItemEventType(parsed.type)) {
    return {
      type: parsed.type,
      ...threadPayload(parsed),
    };
  }

  if (parsed.type === 'screener.pending') {
    return { type: parsed.type };
  }

  if (isThreadEventType(parsed.type)) {
    return {
      type: parsed.type,
      ...threadPayload(parsed),
    };
  }

  return {
    type: parsed.type,
    ...scheduledSendPayload(parsed),
  };
}

export function invalidateQueriesForEvent(
  queryClient: QueryClient,
  event: HailEvent,
) {
  switch (event.type) {
    case 'heartbeat':
      return;
    case 'imbox.new':
      invalidateNewItem(queryClient, 'imbox', event.threadId);
      return;
    case 'feed.new':
      invalidateNewItem(queryClient, 'feed', event.threadId);
      return;
    case 'papertrail.new':
      invalidateNewItem(queryClient, 'papertrail', event.threadId);
      return;
    case 'screener.pending':
      void queryClient.invalidateQueries({ queryKey: queryKeys.screener() });
      return;
    case 'thread.updated':
      invalidateThreadOrAll(queryClient, event.threadId);
      void queryClient.invalidateQueries({ queryKey: queryKeys.views() });
      return;
    case 'thread.removed':
      removeThreadOrInvalidateAll(queryClient, event.threadId);
      void queryClient.invalidateQueries({ queryKey: queryKeys.views() });
      return;
    case 'bubble.fired':
      invalidateThreadOrAll(queryClient, event.threadId);
      void queryClient.invalidateQueries({ queryKey: queryKeys.views() });
      return;
    case 'send.completed':
    case 'send.failed':
      void queryClient.invalidateQueries({ queryKey: queryKeys.threads() });
      void queryClient.invalidateQueries({ queryKey: queryKeys.views() });
      return;
  }
}

function isHailEventType(type: string): type is HailEventType {
  return eventTypes.some((eventType) => eventType === type);
}

function isNewItemEventType(
  type: HailEventType,
): type is 'imbox.new' | 'feed.new' | 'papertrail.new' {
  return type === 'imbox.new' || type === 'feed.new' || type === 'papertrail.new';
}

function isThreadEventType(
  type: HailEventType,
): type is 'thread.updated' | 'thread.removed' | 'bubble.fired' {
  return (
    type === 'thread.updated' || type === 'thread.removed' || type === 'bubble.fired'
  );
}

function threadPayload(parsed: Record<string, unknown>) {
  return typeof parsed.thread_id === 'string'
    ? { threadId: parsed.thread_id }
    : {};
}

function scheduledSendPayload(parsed: Record<string, unknown>) {
  return {
    ...(typeof parsed.scheduled_send_id === 'number'
      ? { scheduledSendId: parsed.scheduled_send_id }
      : {}),
    ...(typeof parsed.error === 'string' ? { error: parsed.error } : {}),
  };
}

function invalidateNewItem(
  queryClient: QueryClient,
  view: 'imbox' | 'feed' | 'papertrail',
  threadId: string | undefined,
) {
  void queryClient.invalidateQueries({ queryKey: queryKeys.view(view) });
  if (threadId !== undefined) {
    void queryClient.invalidateQueries({ queryKey: queryKeys.thread(threadId) });
  }
}

function invalidateThreadOrAll(
  queryClient: QueryClient,
  threadId: string | undefined,
) {
  void queryClient.invalidateQueries({
    queryKey: threadId === undefined ? queryKeys.threads() : queryKeys.thread(threadId),
  });
}

function removeThreadOrInvalidateAll(
  queryClient: QueryClient,
  threadId: string | undefined,
) {
  if (threadId === undefined) {
    void queryClient.invalidateQueries({ queryKey: queryKeys.threads() });
    return;
  }

  queryClient.removeQueries({ queryKey: queryKeys.thread(threadId) });
}

function isObject(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null;
}
