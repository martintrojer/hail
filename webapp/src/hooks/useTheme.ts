import { useEffect, useState, useSyncExternalStore } from 'react';

export type ThemePreference = 'system' | 'light' | 'dark';

const THEME_STORAGE_KEY = 'hail-theme';
const THEME_VALUES = new Set<ThemePreference>(['system', 'light', 'dark']);
const SYSTEM_THEME_QUERY = '(prefers-color-scheme: dark)';

function readStoredTheme(): ThemePreference {
  if (typeof window === 'undefined') {
    return 'system';
  }

  const stored = window.localStorage.getItem(THEME_STORAGE_KEY);
  if (stored !== null && THEME_VALUES.has(stored as ThemePreference)) {
    return stored as ThemePreference;
  }

  return 'system';
}

function applyThemeClass(theme: ThemePreference) {
  if (typeof document === 'undefined') {
    return;
  }

  const root = document.documentElement;
  const systemDark = typeof window !== 'undefined'
    && window.matchMedia(SYSTEM_THEME_QUERY).matches;
  const resolvedDark = theme === 'dark' || (theme === 'system' && systemDark);

  root.classList.toggle('light', theme === 'light');
  root.classList.toggle('dark', resolvedDark);
  root.dataset.theme = theme;
  root.style.colorScheme = resolvedDark ? 'dark' : 'light';
}

function subscribeToSystemTheme(onChange: () => void) {
  if (typeof window === 'undefined') {
    return () => {};
  }

  const media = window.matchMedia(SYSTEM_THEME_QUERY);
  media.addEventListener('change', onChange);

  return () => media.removeEventListener('change', onChange);
}

function getSystemThemeSnapshot() {
  if (typeof window === 'undefined') {
    return false;
  }

  return window.matchMedia(SYSTEM_THEME_QUERY).matches;
}

export function useTheme() {
  const [theme, setThemeState] = useState<ThemePreference>(readStoredTheme);
  const systemDark = useSyncExternalStore(
    subscribeToSystemTheme,
    getSystemThemeSnapshot,
    () => false,
  );

  useEffect(() => {
    applyThemeClass(theme);
    window.localStorage.setItem(THEME_STORAGE_KEY, theme);
  }, [theme, systemDark]);

  return { theme, setTheme: setThemeState };
}
