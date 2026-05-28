import { describe, expect, it } from 'vitest';
import appShellSource from './AppShell.tsx?raw';
import sidebarSource from '../components/ui/sidebar.tsx?raw';
import threadPageSource from '../routes/ThreadPage.tsx?raw';
import composerPageSource from '../routes/ComposerPage.tsx?raw';
import searchPageSource from '../routes/SearchPage.tsx?raw';

const layoutSources = [
  {
    module: 'AppShell',
    source: appShellSource,
  },
  {
    module: 'shadcn Sidebar',
    source: sidebarSource,
  },
];

const routeSources = [
  {
    route: 'ThreadPage',
    source: threadPageSource,
    forbidden: ['mx-auto w-full max-w-3xl', 'lg:max-w-4xl xl:max-w-5xl'],
  },
  {
    route: 'ComposerPage',
    source: composerPageSource,
    forbidden: [
      'mx-auto flex min-h-[calc(100vh-11rem)] w-full max-w-3xl',
      'lg:max-w-4xl xl:max-w-5xl',
    ],
  },
  {
    route: 'SearchPage',
    source: searchPageSource,
    forbidden: ['mx-auto', 'max-w-3xl', 'max-w-4xl', 'max-w-5xl', 'max-w-7xl'],
  },
];

describe('AppShell horizontal overflow guardrails', () => {
  it.each(layoutSources)('$module avoids viewport-width sizing in root layout classes', ({ source }) => {
    expect(source).not.toMatch(/\b(?:w-screen|100vw|vw-)\b/);
  });

  it('allows the Sidebar flex wrapper and inset to shrink instead of forcing document overflow', () => {
    expect(sidebarSource).toContain('group/sidebar-wrapper flex min-h-svh w-full min-w-0 overflow-x-clip');
    expect(sidebarSource).toContain('relative flex min-w-0 flex-1 flex-col bg-background');
  });
});

describe('route AppShell container ownership', () => {
  it.each(routeSources)('$route does not copy AppShell max-width wrappers', ({ source, forbidden }) => {
    for (const fragment of forbidden) {
      expect(source).not.toContain(fragment);
    }
  });
});
