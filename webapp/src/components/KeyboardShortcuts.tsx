import { useEffect, useRef, useState } from 'react';
import { useNavigate } from '@tanstack/react-router';

type ShortcutRoute =
  | '/imbox'
  | '/feed'
  | '/papertrail'
  | '/screener'
  | '/search'
  | '/compose';

const groupTimeoutMs = 1200;

const shortcuts = [
  { keys: '?', action: 'Show keyboard shortcut help' },
  { keys: '/', action: 'Go to Search and focus the search box' },
  { keys: 'Ctrl/Cmd-K', action: 'Go to Search and focus the search box' },
  { keys: 'g then i', action: 'Go to Imbox' },
  { keys: 'g then f', action: 'Go to Feed' },
  { keys: 'g then p', action: 'Go to Paper Trail' },
  { keys: 'g then s', action: 'Go to Screener' },
  { keys: 'c', action: 'Compose (placeholder until composer ships)' },
  { keys: 'j / k', action: 'Move focus through the visible mail list' },
  { keys: 'Esc', action: 'Close this help overlay' },
];

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

function focusSearchInput(attempt = 0) {
  window.setTimeout(() => {
    const input = document.querySelector<HTMLInputElement>(
      '[data-hail-search-input="true"]',
    );
    if (!input && attempt < 10) {
      focusSearchInput(attempt + 1);
      return;
    }

    input?.focus();
    input?.select();
  }, 25);
}

function focusMailListItem(direction: 1 | -1) {
  const items = Array.from(
    document.querySelectorAll<HTMLAnchorElement>('[data-hail-mail-list-item="true"]'),
  ).filter((item) => item.offsetParent !== null);

  if (items.length === 0) {
    // TODO(spa-keyboard): extend list selection once virtualized/active-row state exists.
    return;
  }

  const activeIndex = items.findIndex((item) => item === document.activeElement);
  let nextIndex: number;
  if (activeIndex === -1) {
    nextIndex = direction === 1 ? 0 : items.length - 1;
  } else {
    nextIndex = Math.min(Math.max(activeIndex + direction, 0), items.length - 1);
  }

  items[nextIndex]?.focus();
}

export function KeyboardShortcuts() {
  const navigate = useNavigate();
  const [helpOpen, setHelpOpen] = useState(false);
  const [composeNoticeOpen, setComposeNoticeOpen] = useState(false);
  const pendingGroupRef = useRef<string | null>(null);
  const groupTimerRef = useRef<number | null>(null);
  const composeTimerRef = useRef<number | null>(null);

  useEffect(() => {
    return () => {
      if (groupTimerRef.current !== null) {
        window.clearTimeout(groupTimerRef.current);
      }
      if (composeTimerRef.current !== null) {
        window.clearTimeout(composeTimerRef.current);
      }
    };
  }, []);

  useEffect(() => {
    function navigateTo(to: ShortcutRoute) {
      void navigate({ to });
    }

    function clearPendingGroup() {
      pendingGroupRef.current = null;
      if (groupTimerRef.current !== null) {
        window.clearTimeout(groupTimerRef.current);
        groupTimerRef.current = null;
      }
    }

    function startGroup(key: string) {
      clearPendingGroup();
      pendingGroupRef.current = key;
      groupTimerRef.current = window.setTimeout(clearPendingGroup, groupTimeoutMs);
    }

    function showComposeNotice() {
      setComposeNoticeOpen(true);
      if (composeTimerRef.current !== null) {
        window.clearTimeout(composeTimerRef.current);
      }
      composeTimerRef.current = window.setTimeout(() => {
        setComposeNoticeOpen(false);
        composeTimerRef.current = null;
      }, 3000);
      console.info('Compose shortcut invoked; composer UI is not built yet.');
    }

    function handleKeyDown(event: KeyboardEvent) {
      if (event.defaultPrevented) {
        return;
      }

      if (event.key === 'Escape' && helpOpen) {
        event.preventDefault();
        setHelpOpen(false);
        clearPendingGroup();
        return;
      }

      if (isEditableTarget(event.target)) {
        return;
      }

      const key = event.key.toLowerCase();
      const hasModifier = event.altKey || event.ctrlKey || event.metaKey;

      if ((event.ctrlKey || event.metaKey) && !event.altKey && key === 'k') {
        event.preventDefault();
        navigateTo('/search');
        focusSearchInput();
        clearPendingGroup();
        return;
      }

      if (event.key === '?' && !hasModifier) {
        event.preventDefault();
        setHelpOpen((open) => !open);
        clearPendingGroup();
        return;
      }

      if (event.key === '/' && !hasModifier) {
        event.preventDefault();
        navigateTo('/search');
        focusSearchInput();
        clearPendingGroup();
        return;
      }

      if (key === 'j' && !hasModifier) {
        event.preventDefault();
        focusMailListItem(1);
        clearPendingGroup();
        return;
      }

      if (key === 'k' && !hasModifier) {
        event.preventDefault();
        focusMailListItem(-1);
        clearPendingGroup();
        return;
      }

      if (key === 'c' && !hasModifier) {
        event.preventDefault();
        navigateTo('/compose');
        showComposeNotice();
        clearPendingGroup();
        return;
      }

      if (pendingGroupRef.current === 'g' && !hasModifier) {
        const routeByKey: Record<string, ShortcutRoute | undefined> = {
          i: '/imbox',
          f: '/feed',
          p: '/papertrail',
          s: '/screener',
        };
        const route = routeByKey[key];
        if (route) {
          event.preventDefault();
          navigateTo(route);
          clearPendingGroup();
          return;
        }
      }

      if (key === 'g' && !hasModifier) {
        event.preventDefault();
        startGroup('g');
        return;
      }

      clearPendingGroup();
    }

    window.addEventListener('keydown', handleKeyDown);
    return () => window.removeEventListener('keydown', handleKeyDown);
  }, [helpOpen, navigate]);

  return (
    <>
      {helpOpen ? (
        <div
          className="fixed inset-0 z-50 flex items-center justify-center bg-slate-950/75 p-4 backdrop-blur-sm"
          role="dialog"
          aria-modal="true"
          aria-labelledby="keyboard-shortcuts-title"
          onClick={() => setHelpOpen(false)}
        >
          <section
            className="w-full max-w-lg rounded-3xl border border-slate-700 bg-slate-900 p-6 text-slate-50 shadow-2xl shadow-slate-950"
            onClick={(event) => event.stopPropagation()}
          >
            <div className="flex items-start justify-between gap-4">
              <div>
                <p className="text-xs font-semibold uppercase tracking-[0.3em] text-sky-300">
                  hail
                </p>
                <h2
                  id="keyboard-shortcuts-title"
                  className="mt-2 text-2xl font-semibold tracking-tight"
                >
                  Keyboard shortcuts
                </h2>
              </div>
              <button
                type="button"
                onClick={() => setHelpOpen(false)}
                className="rounded-lg border border-slate-700 px-3 py-1.5 text-sm font-semibold text-slate-200 transition hover:border-sky-400 hover:text-sky-100"
              >
                Esc
              </button>
            </div>

            <dl className="mt-6 divide-y divide-slate-800">
              {shortcuts.map((shortcut) => (
                <div
                  key={shortcut.keys}
                  className="grid grid-cols-[7rem_minmax(0,1fr)] gap-4 py-3"
                >
                  <dt className="font-mono text-sm font-semibold text-sky-100">
                    {shortcut.keys}
                  </dt>
                  <dd className="text-sm leading-6 text-slate-300">
                    {shortcut.action}
                  </dd>
                </div>
              ))}
            </dl>
          </section>
        </div>
      ) : null}

      {composeNoticeOpen ? (
        <div
          role="status"
          className="fixed bottom-4 left-1/2 z-50 -translate-x-1/2 rounded-full border border-slate-700 bg-slate-900 px-4 py-2 text-sm font-medium text-slate-100 shadow-2xl shadow-slate-950"
        >
          Composer is not built yet. Showing the compose placeholder.
        </div>
      ) : null}
    </>
  );
}
