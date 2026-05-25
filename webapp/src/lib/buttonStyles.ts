const focusRing = 'focus-ring outline-none';
const disabled = 'disabled:cursor-not-allowed disabled:opacity-60';

export const primaryPillButtonClass = `rounded-full bg-accent-blue px-3 py-1 text-xs font-semibold text-white ${focusRing} hover:bg-accent-blue-hover ${disabled}`;
export const outlinePillButtonClass = `rounded-full border border-border-menu px-3 py-1 text-xs font-semibold text-ink-secondary ${focusRing} hover:bg-bg-hover hover:text-ink-primary ${disabled}`;

export function pillButtonClass(
  variant: 'primary' | 'outline',
  size: 'sm' | 'md' = 'sm',
) {
  const base = variant === 'primary' ? primaryPillButtonClass : outlinePillButtonClass;
  if (size === 'md') {
    return base.replace('px-3 py-1 text-xs', 'px-4 py-1.5 text-xs');
  }
  return base;
}
