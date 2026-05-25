import { useEffect } from 'react';

export interface KeyboardShortcutHandlers {
  onEscape?: () => void;
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

const escapeHandlers: symbol[] = [];

function registerEscapeHandler(token: symbol) {
  escapeHandlers.push(token);
  return () => {
    const index = escapeHandlers.lastIndexOf(token);
    if (index !== -1) {
      escapeHandlers.splice(index, 1);
    }
  };
}

function isTopEscapeHandler(token: symbol) {
  return escapeHandlers.at(-1) === token;
}

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

interface KeyboardShortcutOptions {
  enabled?: boolean;
}

export function useKeyboardShortcuts(
  handlers: KeyboardShortcutHandlers,
  { enabled = true }: KeyboardShortcutOptions = {},
) {
  useEffect(() => {
    if (!enabled) {
      return undefined;
    }

    const escapeToken = Symbol('hail-keyboard-escape');
    const unregisterEscapeHandler = handlers.onEscape
      ? registerEscapeHandler(escapeToken)
      : undefined;

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
      if (event.defaultPrevented) {
        return;
      }

      if (event.key === 'Escape') {
        if (handlers.onEscape && !isTopEscapeHandler(escapeToken)) {
          return;
        }
        if (run(event, handlers.onEscape)) {
          return;
        }
      }

      if (isEditableTarget(event.target)) {
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
      unregisterEscapeHandler?.();
      clearPendingPrefix();
      window.removeEventListener('keydown', handleKeyDown);
    };
  }, [enabled, handlers]);
}
