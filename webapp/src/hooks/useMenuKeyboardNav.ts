import { useLayoutEffect, useRef } from 'react';

/**
 * Keyboard navigation for popup menus.
 * - j / ArrowDown: focus next menuitem (or first if none focused)
 * - k / ArrowUp: focus previous menuitem (or last if none focused)
 * - Enter: click focused menuitem
 * - Escape: close menu
 *
 * Uses a capture-phase window listener so global shortcuts (useKeyboardShortcuts)
 * cannot steal j/k/Escape while the menu is open.
 *
 * Pass the returned `menuRef` as the `ref` on the menu container element.
 */
export function useMenuKeyboardNav({
  open,
  onClose,
}: {
  open: boolean;
  onClose: () => void;
}) {
  const menuRef = useRef<HTMLElement | null>(null);
  const onCloseRef = useRef(onClose);
  onCloseRef.current = onClose;

  useLayoutEffect(() => {
    if (!open) return undefined;

    function getItems() {
      return Array.from(
        menuRef.current?.querySelectorAll<HTMLElement>('[role=menuitem]') ?? [],
      );
    }

    function currentIndex(items: HTMLElement[]) {
      return items.indexOf(document.activeElement as HTMLElement);
    }

    function handleKeyDown(event: KeyboardEvent) {
      const menu = menuRef.current;
      if (!menu) return;

      const key = event.key;
      const items = getItems();
      if (items.length === 0) return;

      // Navigation keys: intercept if menu is open (focus may or may not be inside)
      if (key === 'j' || key === 'ArrowDown') {
        event.preventDefault();
        event.stopImmediatePropagation();
        const idx = currentIndex(items);
        const next = idx < items.length - 1 ? idx + 1 : 0;
        items[next]?.focus();
        return;
      }

      if (key === 'k' || key === 'ArrowUp') {
        event.preventDefault();
        event.stopImmediatePropagation();
        const idx = currentIndex(items);
        const prev = idx > 0 ? idx - 1 : items.length - 1;
        items[prev]?.focus();
        return;
      }

      if (key === 'Enter') {
        const idx = currentIndex(items);
        if (idx >= 0) {
          event.preventDefault();
          event.stopImmediatePropagation();
          items[idx]?.click();
        }
        return;
      }

      if (key === 'Escape') {
        event.preventDefault();
        event.stopImmediatePropagation();
        onCloseRef.current();
        return;
      }

      // Block all other single-char keys so global shortcuts don't fire
      // while menu is open (e.g. 'c' for compose, 'r' for reply)
      if (key.length === 1 && !event.ctrlKey && !event.metaKey && !event.altKey) {
        event.stopImmediatePropagation();
      }
    }

    window.addEventListener('keydown', handleKeyDown, true);
    return () => window.removeEventListener('keydown', handleKeyDown, true);
  }, [open]);

  return menuRef;
}
