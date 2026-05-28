import { useEffect, useRef } from 'react';
import { useKeyboardShortcuts } from '../hooks/useKeyboardShortcuts';
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from './ui/dialog';

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
      { keys: ['g', 'g'], description: 'First thread' },
      { keys: ['G'], description: 'Last thread' },
      { keys: ['Ctrl', 'd'], description: 'Half page down', combo: true },
      { keys: ['Ctrl', 'u'], description: 'Half page up', combo: true },
      { keys: ['o'], description: 'Open thread' },
      { keys: ['m'], description: 'Toggle menu' },
      { keys: ['g', 'i'], description: 'Go to Imbox' },
      { keys: ['g', 'f'], description: 'Go to Feed' },
      { keys: ['g', 'p'], description: 'Go to Paper Trail' },
      { keys: ['g', 's'], description: 'Go to Screener' },
      { keys: ['g', 'd'], description: 'Go to Drafts' },
      { keys: ['g', 'j'], description: 'Go to Spam' },
      { keys: ['g', 't'], description: 'Go to Trash' },
      { keys: ['g', 'a'], description: 'Go to Set Aside' },
      { keys: ['g', 'l'], description: 'Go to Reply Later' },
      { keys: ['g', 'b'], description: 'Go to Bubble Up' },
      { keys: ['g', 'r'], description: 'Go to Archive' },
      { keys: ['/'], description: 'Focus search' },
      { keys: ['c'], description: 'Compose' },
    ],
  },
  {
    title: 'Thread actions',
    shortcuts: [
      { keys: ['d'], description: 'Trash' },
      { keys: ['e'], description: 'Archive' },
      { keys: ['y'], description: 'Set aside' },
      { keys: ['l'], description: 'Reply later' },
      { keys: ['r'], description: 'Reply' },
      { keys: ['a'], description: 'Reply all' },
      { keys: ['f'], description: 'Forward' },
      { keys: ['n'], description: 'Add note' },
      { keys: ['x'], description: 'Select / deselect' },
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
    <kbd className="inline-flex min-w-6 items-center justify-center rounded-md border bg-muted px-1.5 py-0.5 font-mono text-xs font-medium leading-tight text-foreground shadow-xs">
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
    <Dialog modal={false} open={open} onOpenChange={(nextOpen) => {
      if (!nextOpen) {
        onClose();
      }
    }}>
      <DialogContent
        aria-describedby={undefined}
        aria-label="Keyboard Shortcuts"
        className="max-h-[min(42rem,calc(100vh-2rem))] overflow-y-auto sm:max-w-2xl"
        showCloseButton={false}
      >
        <DialogHeader>
          <DialogTitle id="keyboard-shortcuts-title" className="sr-only">Keyboard Shortcuts</DialogTitle>
          <h2 aria-hidden="true" className="font-heading text-base leading-none font-medium">
            Keyboard Shortcuts
          </h2>
        </DialogHeader>

        <button
          ref={closeButtonRef}
          type="button"
          onClick={onClose}
          className="sr-only"
        >
          Close
        </button>

        <div className="grid gap-6 sm:grid-cols-2">
          {shortcutGroups.map((group) => (
            <section key={group.title} aria-labelledby={`shortcut-group-${group.title}`}>
              <h3
                id={`shortcut-group-${group.title}`}
                className="text-sm font-semibold text-muted-foreground"
              >
                {group.title}
              </h3>
              <dl className="mt-3 flex flex-col gap-3">
                {group.shortcuts.map((shortcut) => (
                  <div
                    key={`${shortcut.keys.join('-')}-${shortcut.description}`}
                    className="grid grid-cols-[8rem_minmax(0,1fr)] items-center gap-3"
                  >
                    <dt className="flex items-center gap-1">
                      {shortcut.keys.map((key, index) => (
                        <span key={`${key}-${index}`} className="flex items-center gap-1">
                          {index > 0 ? (
                            <span className="text-xs text-muted-foreground">{shortcut.combo ? '+' : 'then'}</span>
                          ) : null}
                          <Keycap>{key}</Keycap>
                        </span>
                      ))}
                    </dt>
                    <dd className="text-sm leading-6 text-muted-foreground">
                      {shortcut.description}
                    </dd>
                  </div>
                ))}
              </dl>
            </section>
          ))}
        </div>
      </DialogContent>
    </Dialog>
  );
}
