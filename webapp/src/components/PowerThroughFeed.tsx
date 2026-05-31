import { useEffect, useMemo, useRef, useState } from 'react';
import { useNavigate } from '@tanstack/react-router';
import { cn } from '../lib/utils';
import type { FeedBlockedTracker, HailApiClient, MailViewItem } from '../api/client';
import {
  useArchiveThreadMutation,
  useClassifyThreadMutation,
  useReplyLaterThreadMutation,
  useSetAsideThreadMutation,
  useTrashThreadMutation,
} from '../api/query';
import { useKeyboardShortcuts } from '../hooks/useKeyboardShortcuts';
import { actionErrorMessage } from '../lib/errorMessages';
import { EmailFrame } from './EmailFrame';
import { MailRowQuickActionsMenu } from './MailRow';
import { useUndoToast } from './UndoToastProvider';
import { Badge } from './ui/badge';
import { Button } from './ui/button';
import { Card, CardAction, CardContent, CardDescription, CardHeader, CardTitle } from './ui/card';
import { StateCard } from './StateCard';

const POWER_THROUGH_COLLAPSED_MAX_HEIGHT = 600;
const POWER_THROUGH_READ_DWELL_MS = 800;

type PowerThroughAction = 'imbox' | 'feed' | 'papertrail' | 'archive' | 'set-aside' | 'reply-later' | 'trash';

const targetLabels: Record<Extract<PowerThroughAction, 'imbox' | 'feed' | 'papertrail'>, string> = {
  imbox: 'Imbox',
  feed: 'Feed',
  papertrail: 'Paper Trail',
};

function TrackerSummary({ trackers }: { trackers: FeedBlockedTracker[] }) {
  if (trackers.length === 0) return null;
  return (
    <Badge variant="secondary" title={trackers.map((tracker) => tracker.reason).join('\n')}>
      {trackers.length} tracker{trackers.length === 1 ? '' : 's'} blocked
    </Badge>
  );
}

function likelyLongHtml(html: string) {
  return html.length > 1800;
}

function undoFrom(data: unknown) {
  const undoable = data as { undo?: { id: string } } | undefined;
  return undoable?.undo ? { id: undoable.undo.id } : null;
}

function useScrollPastSeenObserver({ items, client, onSeen }: { items: MailViewItem[]; client: HailApiClient; onSeen: (threadId: string) => void }) {
  const markedRef = useRef<Set<string>>(new Set());
  const elementsRef = useRef<Map<string, HTMLElement>>(new Map());
  const dwellStartRef = useRef<Map<string, number>>(new Map());
  const offTopRef = useRef<Set<string>>(new Set());
  const timersRef = useRef<Map<string, number>>(new Map());
  const [errors, setErrors] = useState<Record<string, Error>>({});

  useEffect(() => {
    const validIds = new Set(items.map((item) => item.thread_id));
    markedRef.current = new Set([...markedRef.current].filter((id) => validIds.has(id)));
    dwellStartRef.current = new Map([...dwellStartRef.current].filter(([id]) => validIds.has(id)));
    for (const [id, timer] of timersRef.current) {
      if (!validIds.has(id)) {
        window.clearTimeout(timer);
        timersRef.current.delete(id);
        offTopRef.current.delete(id);
      }
    }
  }, [items]);

  useEffect(() => () => {
    for (const timer of timersRef.current.values()) window.clearTimeout(timer);
    timersRef.current.clear();
  }, []);

  useEffect(() => {
    if (typeof IntersectionObserver === 'undefined') return undefined;

    function markSeen(threadId: string) {
      if (markedRef.current.has(threadId)) return;
      markedRef.current.add(threadId);
      offTopRef.current.delete(threadId);
      const existingTimer = timersRef.current.get(threadId);
      if (existingTimer !== undefined) {
        window.clearTimeout(existingTimer);
        timersRef.current.delete(threadId);
      }
      void client.markThread(threadId, true)
        .then(() => onSeen(threadId))
        .catch((error: Error) => {
          markedRef.current.delete(threadId);
          setErrors((current) => ({ ...current, [threadId]: error }));
        });
    }

    function scheduleMark(threadId: string) {
      if (markedRef.current.has(threadId)) return;
      offTopRef.current.add(threadId);
      const startedAt = dwellStartRef.current.get(threadId) ?? Date.now();
      dwellStartRef.current.set(threadId, startedAt);
      const remaining = POWER_THROUGH_READ_DWELL_MS - (Date.now() - startedAt);
      const existingTimer = timersRef.current.get(threadId);
      if (existingTimer !== undefined) window.clearTimeout(existingTimer);
      if (remaining <= 0) {
        markSeen(threadId);
        return;
      }
      const timer = window.setTimeout(() => {
        timersRef.current.delete(threadId);
        if (offTopRef.current.has(threadId)) markSeen(threadId);
      }, remaining);
      timersRef.current.set(threadId, timer);
    }

    const observer = new IntersectionObserver((entries) => {
      for (const entry of entries) {
        const threadId = (entry.target as HTMLElement).dataset.hailThreadId;
        if (!threadId || markedRef.current.has(threadId)) continue;
        if (entry.isIntersecting) {
          if (!dwellStartRef.current.has(threadId)) dwellStartRef.current.set(threadId, Date.now());
          offTopRef.current.delete(threadId);
          const existingTimer = timersRef.current.get(threadId);
          if (existingTimer !== undefined) {
            window.clearTimeout(existingTimer);
            timersRef.current.delete(threadId);
          }
          continue;
        }
        if (entry.boundingClientRect.bottom <= 0) scheduleMark(threadId);
      }
    }, { threshold: 0 });

    for (const element of elementsRef.current.values()) observer.observe(element);
    return () => observer.disconnect();
  }, [client, items, onSeen]);

  function register(threadId: string) {
    return (element: HTMLElement | null) => {
      if (element) {
        elementsRef.current.set(threadId, element);
        if (!dwellStartRef.current.has(threadId)) {
          dwellStartRef.current.set(threadId, Date.now());
        }
      } else {
        elementsRef.current.delete(threadId);
      }
    };
  }

  return { errors, register };
}

function PowerThroughMessageCard({ item, active, busy, error, markError, register, onActivate, onAction }: {
  item: MailViewItem;
  active: boolean;
  busy: boolean;
  error: Error | null;
  markError?: Error;
  register: (threadId: string) => (element: HTMLElement | null) => void;
  onActivate: () => void;
  onAction: (threadId: string, action: PowerThroughAction) => void;
}) {
  const [expanded, setExpanded] = useState(false);
  const [measuredHeight, setMeasuredHeight] = useState(0);
  const html = item.feed_html?.trim() || '';
  const trackers = item.feed_blocked_trackers ?? [];
  const shouldClamp = measuredHeight > POWER_THROUGH_COLLAPSED_MAX_HEIGHT || likelyLongHtml(html);
  const clamped = shouldClamp && !expanded;
  const actionLabel = (label: string) => active ? label : `${label} for ${item.subject || '(no subject)'}`;

  return (
    <article
      ref={register(item.thread_id)}
      data-hail-thread-id={item.thread_id}
      data-hail-mail-list-item="true"
      tabIndex={-1}
      aria-current={active ? 'true' : undefined}
      className="scroll-mt-16 outline-none"
      onFocus={onActivate}
      onClick={onActivate}
    >
      <Card size="sm" className={cn('gap-0 border border-border py-0 shadow-none ring-0 transition-shadow', active && 'border-primary shadow-sm ring-1 ring-primary/40')}>
        <CardHeader className="border-b border-border p-4 pb-3 sm:p-5 sm:pb-3">
          <div className="min-w-0">
            <CardDescription className="truncate font-medium">{item.from || 'Unknown sender'}</CardDescription>
            <CardTitle className="mt-1 text-lg font-semibold tracking-tight text-foreground">{item.subject || '(no subject)'}</CardTitle>
          </div>
          <CardAction className="flex flex-col items-end gap-2">
            {item.unread ? <Badge>New</Badge> : null}
            <TrackerSummary trackers={trackers} />
            <MailRowQuickActionsMenu threadId={item.thread_id} subject={item.subject || '(no subject)'} unread={item.unread} selected={false} />
          </CardAction>
        </CardHeader>

        <CardContent className="p-4 sm:p-5">
          {html.length > 0 ? (
            <div className="relative">
              <div
                className={cn('relative overflow-hidden', clamped && 'after:pointer-events-none after:absolute after:inset-x-0 after:bottom-0 after:h-24 after:bg-gradient-to-b after:from-transparent after:to-card')}
                style={clamped ? { maxHeight: POWER_THROUGH_COLLAPSED_MAX_HEIGHT } : undefined}
              >
                <EmailFrame html={html} title={`Email body from ${item.from || 'Unknown sender'}`} onHeightChange={setMeasuredHeight} />
              </div>
              {clamped ? (
                <div className="mt-4 flex justify-center">
                  <Button type="button" variant="outline" onClick={() => setExpanded(true)}>Show full message</Button>
                </div>
              ) : null}
            </div>
          ) : (
            <p className="whitespace-pre-wrap text-base leading-relaxed text-foreground">{item.preview || 'This message has no renderable body.'}</p>
          )}

          <div className="mt-5 flex flex-wrap gap-2" aria-label={`Quick actions for ${item.subject || '(no subject)'}`}>
            <Button type="button" aria-label={actionLabel('Keep in Imbox')} disabled={busy} onClick={() => onAction(item.thread_id, 'imbox')} size="sm">Imbox</Button>
            <Button type="button" aria-label={actionLabel('Move to Feed')} disabled={busy} onClick={() => onAction(item.thread_id, 'feed')} variant="outline" size="sm">Feed</Button>
            <Button type="button" aria-label={actionLabel('Move to Paper Trail')} disabled={busy} onClick={() => onAction(item.thread_id, 'papertrail')} variant="outline" size="sm">Papertrail</Button>
            <Button type="button" aria-label={actionLabel('Set Aside')} disabled={busy} onClick={() => onAction(item.thread_id, 'set-aside')} variant="outline" size="sm">Set Aside</Button>
            <Button type="button" aria-label={actionLabel('Reply Later')} disabled={busy} onClick={() => onAction(item.thread_id, 'reply-later')} variant="outline" size="sm">Reply Later</Button>
            <Button type="button" aria-label={actionLabel('Trash')} disabled={busy} onClick={() => onAction(item.thread_id, 'trash')} variant="destructive" size="sm">Trash</Button>
          </div>

          {error ? <p role="alert" className="mt-3 text-sm text-destructive">{actionErrorMessage(error, 'Thread action')}</p> : null}
          {markError ? <p role="alert" className="mt-3 text-sm text-destructive">{actionErrorMessage(markError, 'Mark read')}</p> : null}
        </CardContent>
      </Card>
    </article>
  );
}

export function PowerThroughFeed({ items, client, onExit }: { items: MailViewItem[]; client: HailApiClient; onExit: () => void }) {
  const navigate = useNavigate();
  const undoToast = useUndoToast();
  const [activeIndex, setActiveIndex] = useState(0);
  const [completedThreadIds, setCompletedThreadIds] = useState<Set<string>>(new Set());
  const activeIndexRef = useRef(0);
  const [actionError, setActionError] = useState<Error | null>(null);
  const initialTotalRef = useRef(items.filter((item) => item.unread).length);
  const cardRefs = useRef<Map<string, HTMLElement>>(new Map());
  const classify = useClassifyThreadMutation(client);
  const archive = useArchiveThreadMutation(client);
  const setAside = useSetAsideThreadMutation(client);
  const replyLater = useReplyLaterThreadMutation(client);
  const trash = useTrashThreadMutation(client);

  const visibleItems = useMemo(() => items.filter((item) => item.unread && !completedThreadIds.has(item.thread_id)), [items, completedThreadIds]);
  const busy = classify.isPending || archive.isPending || setAside.isPending || replyLater.isPending || trash.isPending;
  const mutationError = actionError ?? classify.error ?? archive.error ?? setAside.error ?? replyLater.error ?? trash.error;

  useEffect(() => {
    setCompletedThreadIds((current) => {
      const validIds = new Set(items.map((item) => item.thread_id));
      const next = new Set([...current].filter((id) => validIds.has(id)));
      return next.size === current.size ? current : next;
    });
  }, [items]);

  useEffect(() => setActiveIndex((index) => Math.min(index, Math.max(visibleItems.length - 1, 0))), [visibleItems.length]);

  useEffect(() => {
    const item = visibleItems[activeIndex];
    if (item) cardRefs.current.get(item.thread_id)?.focus();
  }, [activeIndex, visibleItems]);

  function completeThread(threadId: string) {
    setCompletedThreadIds((current) => new Set(current).add(threadId));
  }

  const { errors: markErrors, register: registerSeen } = useScrollPastSeenObserver({ items: visibleItems, client, onSeen: completeThread });

  function registerCard(threadId: string) {
    const seenRegister = registerSeen(threadId);
    return (element: HTMLElement | null) => {
      seenRegister(element);
      if (element) cardRefs.current.set(threadId, element);
      else cardRefs.current.delete(threadId);
    };
  }

  function scrollToIndex(index: number) {
    const nextIndex = Math.min(Math.max(index, 0), visibleItems.length - 1);
    setActiveIndex(nextIndex);
    const item = visibleItems[nextIndex];
    if (item) {
      const element = cardRefs.current.get(item.thread_id);
      element?.scrollIntoView({ block: 'start', behavior: 'smooth' });
      element?.focus();
    }
  }

  activeIndexRef.current = activeIndex;
  const activeItem = visibleItems[activeIndex] ?? null;

  async function handleAction(threadId: string, action: PowerThroughAction) {
    if (busy) return;
    setActionError(null);
    try {
      let data: unknown;
      let message = '';
      let undoSuccessMessage: string | undefined;
      if (action === 'imbox') {
        await client.markThread(threadId, true);
        message = 'Kept thread in Imbox.';
      } else if (action === 'feed' || action === 'papertrail') {
        data = await classify.mutateAsync({ threadId, to: action });
        message = `Moved thread to ${targetLabels[action]}.`;
        undoSuccessMessage = 'Thread classification undone.';
      } else if (action === 'archive') {
        data = await archive.mutateAsync({ threadId });
        message = 'Thread archived.';
        undoSuccessMessage = 'Archive undone.';
      } else if (action === 'set-aside') {
        data = await setAside.mutateAsync({ threadId });
        message = 'Thread added to Set Aside.';
        undoSuccessMessage = 'Set Aside undone.';
      } else if (action === 'reply-later') {
        data = await replyLater.mutateAsync({ threadId });
        message = 'Thread added to Reply Later.';
        undoSuccessMessage = 'Reply Later undone.';
      } else {
        data = await trash.mutateAsync({ threadId });
        message = 'Thread moved to trash.';
        undoSuccessMessage = 'Trash undone.';
      }
      undoToast.showToast({ message, undo: undoFrom(data), undoSuccessMessage });
      completeThread(threadId);
    } catch (error) {
      setActionError(error instanceof Error ? error : new Error('Thread action failed'));
    }
  }


  useEffect(() => {
    function handleCardNavigation(event: KeyboardEvent) {
      if (event.defaultPrevented || event.altKey || event.ctrlKey || event.metaKey) return;
      const target = event.target;
      if (
        target instanceof HTMLInputElement ||
        target instanceof HTMLTextAreaElement ||
        target instanceof HTMLSelectElement ||
        (target instanceof HTMLElement && target.isContentEditable)
      ) {
        return;
      }
      if (event.key === 'j') {
        event.preventDefault();
        scrollToIndex(activeIndexRef.current + 1);
      } else if (event.key === 'k') {
        event.preventDefault();
        scrollToIndex(activeIndexRef.current - 1);
      }
    }

    window.addEventListener('keydown', handleCardNavigation, { capture: true });
    return () => window.removeEventListener('keydown', handleCardNavigation, { capture: true });
  });

  useKeyboardShortcuts({
    onEscape: onExit,
    onNextThread: () => scrollToIndex(activeIndexRef.current + 1),
    onPreviousThread: () => scrollToIndex(activeIndexRef.current - 1),
    onReply: () => { if (activeItem) void navigate({ to: '/compose', search: { replyTo: activeItem.thread_id, replyAll: false, in_reply_to: activeItem.email_id } }); },
    onReplyAll: () => { if (activeItem) void navigate({ to: '/compose', search: { replyTo: activeItem.thread_id, replyAll: true, in_reply_to: activeItem.email_id } }); },
    onForward: () => { if (activeItem) void navigate({ to: '/compose', search: { forward: activeItem.thread_id, in_reply_to: activeItem.email_id } }); },
    onArchive: () => { if (activeItem) void handleAction(activeItem.thread_id, 'archive'); },
    onTrash: () => { if (activeItem) void handleAction(activeItem.thread_id, 'trash'); },
    onSetAside: () => { if (activeItem) void handleAction(activeItem.thread_id, 'set-aside'); },
    onReplyLater: () => { if (activeItem) void handleAction(activeItem.thread_id, 'reply-later'); },
    onOpenActionMenu: () => {
      if (!activeItem) return;
      cardRefs.current.get(activeItem.thread_id)?.focus();
      window.dispatchEvent(new CustomEvent('hail:mail-shortcut', { detail: { action: 'open-menu' } }));
    },
  });

  if (visibleItems.length === 0) {
    return (
      <div className="flex flex-col gap-4">
        <StateCard title="You're all caught up." body="New mail will appear here." />
        <span className="sr-only">All done!</span>
        <div className="flex justify-end">
          <Button type="button" variant="outline" onClick={onExit}>Exit</Button>
        </div>
      </div>
    );
  }

  return (
    <div className="flex flex-col gap-4 sm:gap-5">
      <div className="flex items-center justify-between gap-3 px-1">
        <div>
          <p className="text-sm font-medium text-foreground">Power Through</p>
          <p className="text-xs text-muted-foreground">{completedThreadIds.size + activeIndex + 1} of {initialTotalRef.current}</p>
        </div>
        <Button type="button" variant="ghost" size="sm" onClick={onExit}>Exit</Button>
      </div>
      {visibleItems.map((item, index) => (
        <PowerThroughMessageCard
          key={item.thread_id}
          item={item}
          active={index === activeIndex}
          busy={busy}
          error={index === activeIndex ? mutationError : null}
          markError={markErrors[item.thread_id]}
          register={registerCard}
          onActivate={() => setActiveIndex(index)}
          onAction={(threadId, action) => void handleAction(threadId, action)}
        />
      ))}
    </div>
  );
}
