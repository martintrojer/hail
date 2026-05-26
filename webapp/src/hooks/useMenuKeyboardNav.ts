import { useEffect, type RefObject, type KeyboardEvent as ReactKeyboardEvent } from 'react';

/**
 * Keyboard navigation for popup menus.
 * - j / ArrowDown: focus next menuitem
 * - k / ArrowUp: focus previous menuitem
 * - Enter: click focused menuitem
 * - Escape: close menu
 *
 * Also captures these keys at the window level (capture phase) so global
 * shortcuts don't steal them while focus is inside the menu.
 */
export function useMenuKeyboardNav({
  menuRef,
  open,
  autoFocus = true,
}: {
  menuRef: RefObject<HTMLElement | null>;
  open: boolean;
  /** Focus the first menuitem when the menu opens. Default true. */
  autoFocus?: boolean;
}) {
  // Auto-focus first item on open
  useEffect(() => {
    if (!open || !autoFocus) return;
    requestAnimationFrame(() => {
      const first = menuRef.current?.querySelector<HTMLElement>('[role=menuitem]');
      first?.focus();
    });
  }, [open, autoFocus, menuRef]);

  // Capture j/k/arrows at window level so global shortcuts don't intercept
  useEffect(() => {
    if (!open) return undefined;

    function captureKeys(event: KeyboardEvent) {
      if (!menuRef.current?.contains(document.activeElement)) return;
      const key = event.key;
      if (
        key === 'j' ||
        key === 'k' ||
        key === 'ArrowDown' ||
        key === 'ArrowUp' ||
        key === 'Enter' ||
        key === 'Escape'
      ) {
        event.stopImmediatePropagation();
      }
    }

    window.addEventListener('keydown', captureKeys, true);
    return () => window.removeEventListener('keydown', captureKeys, true);
  }, [open, menuRef]);
}

/** React onKeyDown handler for a menu container. Attach to the menu div. */
export function menuKeyDownHandler(
  menuRef: RefObject<HTMLElement | null>,
  onClose: () => void,
  event: ReactKeyboardEvent,
) {
  const items = menuRef.current?.querySelectorAll<HTMLElement>('[role=menuitem]');
  if (!items?.length) return;

  const current = Array.from(items).indexOf(document.activeElement as HTMLElement);

  if (event.key === 'ArrowDown' || event.key === 'j') {
    event.preventDefault();
    event.stopPropagation();
    const next = current < items.length - 1 ? current + 1 : 0;
    items[next]?.focus();
  } else if (event.key === 'ArrowUp' || event.key === 'k') {
    event.preventDefault();
    event.stopPropagation();
    const prev = current > 0 ? current - 1 : items.length - 1;
    items[prev]?.focus();
  } else if (event.key === 'Enter' && current >= 0) {
    event.preventDefault();
    event.stopPropagation();
    items[current]?.click();
  } else if (event.key === 'Escape') {
    event.preventDefault();
    event.stopPropagation();
    onClose();
  }
}
