import { useEffect, useRef } from 'react';
import { useKeyboardShortcuts } from '../hooks/useKeyboardShortcuts';

interface ShortcutGroup {
  title: string;
  shortcuts: Array<{ keys: string[]; description: string; combo?: boolean }>;
}

const shortcutGroups: ShortcutGroup[] = [
  {
    title: 'Navigation',
    shortcuts: [
      { keys: ['j'], description: 'Next thread' },
      { keys: ['k'], description: 'Previous thread' },
      { keys: ['g', 'i'], description: 'Go to Imbox' },
      { keys: ['g', 'f'], description: 'Go to Feed' },
      { keys: ['g', 'p'], description: 'Go to Paper Trail' },
      { keys: ['g', 's'], description: 'Go to Screener' },
      { keys: ['g', 'd'], description: 'Go to Drafts' },
      { keys: ['g', 't'], description: 'Go to Trash' },
      { keys: ['g', 'a'], description: 'Go to Set Aside' },
      { keys: ['g', 'l'], description: 'Go to Reply Later' },
      { keys: ['g', 'b'], description: 'Go to Bubble Up' },
      { keys: ['/'], description: 'Focus search' },
      { keys: ['c'], description: 'Compose' },
    ],
  },
  {
    title: 'Thread actions',
    shortcuts: [
      { keys: ['e'], description: 'Archive' },
      { keys: ['#'], description: 'Trash' },
      { keys: ['y'], description: 'Set aside' },
      { keys: ['l'], description: 'Reply later' },
      { keys: ['r'], description: 'Reply' },
      { keys: ['a'], description: 'Reply all' },
      { keys: ['f'], description: 'Forward' },
      { keys: ['n'], description: 'Add note' },
      { keys: ['Backspace'], description: 'Go back' },
      { keys: ['?'], description: 'Show shortcut help' },
      { keys: ['Esc'], description: 'Close overlay or go back' },
    ],
  },
  {
    title: 'Composer',
    shortcuts: [
      { keys: ['Ctrl', 'Enter'], description: 'Send', combo: true },
      { keys: ['Esc'], description: 'Close composer' },
    ],
  },
];

function Keycap({ children }: { children: string }) {
  return (
    <kbd className="inline-block rounded border border-border-hairline bg-bg-hover px-1.5 py-0.5 font-mono text-sm font-normal leading-tight text-ink-primary">
      {children}
    </kbd>
  );
}

export function KeyboardShortcutHelp({
  open,
  onClose,
}: {
  open: boolean;
  onClose: () => void;
}) {
  const closeButtonRef = useRef<HTMLButtonElement | null>(null);

  useEffect(() => {
    if (!open) {
      return undefined;
    }

    closeButtonRef.current?.focus();
  }, [open]);

  useKeyboardShortcuts(
    { onEscape: onClose },
    { enabled: open },
  );

  if (!open) {
    return null;
  }

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/40 p-4"
      role="dialog"
      aria-modal="true"
      aria-labelledby="keyboard-shortcuts-title"
      onClick={onClose}
    >
      <section
        className="w-full max-w-2xl rounded-lg bg-bg-surface p-6 text-ink-primary shadow-lg"
        onClick={(event) => event.stopPropagation()}
      >
        <div className="flex items-start justify-between gap-4">
          <h2 id="keyboard-shortcuts-title" className="text-lg font-bold">
            Keyboard Shortcuts
          </h2>
          <button
            ref={closeButtonRef}
            type="button"
            onClick={onClose}
            className="rounded-md px-2 py-1 text-sm font-semibold text-ink-secondary focus-ring outline-none hover:bg-bg-hover hover:text-ink-primary"
          >
            Close
          </button>
        </div>

        <div className="mt-6 grid gap-6 sm:grid-cols-2">
          {shortcutGroups.map((group) => (
            <section key={group.title} aria-labelledby={`shortcut-group-${group.title}`}>
              <h3
                id={`shortcut-group-${group.title}`}
                className="text-sm font-semibold text-ink-secondary"
              >
                {group.title}
              </h3>
              <dl className="mt-3 space-y-3">
                {group.shortcuts.map((shortcut) => (
                  <div
                    key={`${shortcut.keys.join('-')}-${shortcut.description}`}
                    className="grid grid-cols-[5.5rem_minmax(0,1fr)] items-center gap-3"
                  >
                    <dt className="flex items-center gap-1">
                      {shortcut.keys.map((key, index) => (
                        <span key={`${key}-${index}`} className="flex items-center gap-1">
                          {index > 0 ? (
                            <span className="text-xs text-ink-tertiary">{shortcut.combo ? '+' : 'then'}</span>
                          ) : null}
                          <Keycap>{key}</Keycap>
                        </span>
                      ))}
                    </dt>
                    <dd className="text-sm leading-6 text-ink-secondary">
                      {shortcut.description}
                    </dd>
                  </div>
                ))}
              </dl>
            </section>
          ))}
        </div>
      </section>
    </div>
  );
}
