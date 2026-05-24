import { useEffect } from 'react';

export interface KeyboardShortcutHandlers {
  onNextThread?: () => void;
  onPreviousThread?: () => void;
  onArchive?: () => void;
  onTrash?: () => void;
  onSetAside?: () => void;
  onReplyLater?: () => void;
  onReply?: () => void;
  onCompose?: () => void;
  onFocusSearch?: () => void;
  onGoImbox?: () => void;
  onGoFeed?: () => void;
  onGoPaperTrail?: () => void;
  onGoScreener?: () => void;
  onShowHelp?: () => void;
}

const sequenceTimeoutMs = 1200;

function isEditableTarget(target: EventTarget | null) {
  if (!(target instanceof HTMLElement)) {
    return false;
  }

  return (
    target instanceof HTMLInputElement ||
    target instanceof HTMLTextAreaElement ||
    target instanceof HTMLSelectElement ||
    target.isContentEditable
  );
}

export function useKeyboardShortcuts(handlers: KeyboardShortcutHandlers) {
  useEffect(() => {
    let pendingPrefix: 'g' | null = null;
    let prefixTimer: number | null = null;

    function clearPendingPrefix() {
      pendingPrefix = null;
      if (prefixTimer !== null) {
        window.clearTimeout(prefixTimer);
        prefixTimer = null;
      }
    }

    function startPrefix(prefix: 'g') {
      clearPendingPrefix();
      pendingPrefix = prefix;
      prefixTimer = window.setTimeout(clearPendingPrefix, sequenceTimeoutMs);
    }

    function run(event: KeyboardEvent, handler: (() => void) | undefined) {
      if (!handler) {
        return false;
      }

      event.preventDefault();
      handler();
      clearPendingPrefix();
      return true;
    }

    function handleKeyDown(event: KeyboardEvent) {
      if (event.defaultPrevented || isEditableTarget(event.target)) {
        return;
      }

      const key = event.key.toLowerCase();
      const hasModifier = event.altKey || event.ctrlKey || event.metaKey;

      if (hasModifier) {
        clearPendingPrefix();
        return;
      }

      if (pendingPrefix === 'g') {
        const routeHandlers: Record<string, (() => void) | undefined> = {
          i: handlers.onGoImbox,
          f: handlers.onGoFeed,
          p: handlers.onGoPaperTrail,
          s: handlers.onGoScreener,
        };
        if (run(event, routeHandlers[key])) {
          return;
        }
      }

      if (key === 'g') {
        event.preventDefault();
        startPrefix('g');
        return;
      }

      const shortcutHandlers: Record<string, (() => void) | undefined> = {
        j: handlers.onNextThread,
        k: handlers.onPreviousThread,
        e: handlers.onArchive,
        '#': handlers.onTrash,
        y: handlers.onSetAside,
        l: handlers.onReplyLater,
        r: handlers.onReply,
        c: handlers.onCompose,
        '/': handlers.onFocusSearch,
        '?': handlers.onShowHelp,
      };

      if (run(event, shortcutHandlers[event.key] ?? shortcutHandlers[key])) {
        return;
      }

      clearPendingPrefix();
    }

    window.addEventListener('keydown', handleKeyDown);
    return () => {
      clearPendingPrefix();
      window.removeEventListener('keydown', handleKeyDown);
    };
  }, [handlers]);
}
