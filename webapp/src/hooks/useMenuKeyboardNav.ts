import { useEffect, type RefObject } from 'react';

/**
 * Keyboard navigation for popup menus.
 * - j / ArrowDown: focus next menuitem
 * - k / ArrowUp: focus previous menuitem
 * - Enter: click focused menuitem
 * - Escape: close menu
 *
 * Uses a capture-phase window listener so global shortcuts (useKeyboardShortcuts)
 * cannot steal j/k/Escape while focus is inside the menu.
 */
export function useMenuKeyboardNav({
  menuRef,
  open,
  onClose,
  autoFocus = true,
}: {
  menuRef: RefObject<HTMLElement | null>;
  open: boolean;
  onClose: () => void;
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

  // Single capture-phase listener handles both navigation and blocking global shortcuts
  useEffect(() => {
    if (!open) return undefined;

    function handleKeyDown(event: KeyboardEvent) {
      const menu = menuRef.current;
      if (!menu) return;

      // Only act when focus is inside the menu OR the menu just opened
      const focusInMenu = menu.contains(document.activeElement);
      if (!focusInMenu) return;

      const items = Array.from(menu.querySelectorAll<HTMLElement>('[role=menuitem]'));
      if (items.length === 0) return;

      const current = items.indexOf(document.activeElement as HTMLElement);
      const key = event.key;

      if (key === 'ArrowDown' || key === 'j') {
        event.preventDefault();
        event.stopImmediatePropagation();
        const next = current < items.length - 1 ? current + 1 : 0;
        items[next]?.focus();
      } else if (key === 'ArrowUp' || key === 'k') {
        event.preventDefault();
        event.stopImmediatePropagation();
        const prev = current > 0 ? current - 1 : items.length - 1;
        items[prev]?.focus();
      } else if (key === 'Enter' && current >= 0) {
        event.preventDefault();
        event.stopImmediatePropagation();
        items[current]?.click();
      } else if (key === 'Escape') {
        event.preventDefault();
        event.stopImmediatePropagation();
        onClose();
      }
    }

    window.addEventListener('keydown', handleKeyDown, true);
    return () => window.removeEventListener('keydown', handleKeyDown, true);
  }, [open, menuRef, onClose]);
}
