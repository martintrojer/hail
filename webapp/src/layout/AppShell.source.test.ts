import { describe, expect, it } from 'vitest';
import threadPageSource from '../routes/ThreadPage.tsx?raw';
import composerPageSource from '../routes/ComposerPage.tsx?raw';
import searchPageSource from '../routes/SearchPage.tsx?raw';

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

describe('route AppShell container ownership', () => {
  it.each(routeSources)('$route does not copy AppShell max-width wrappers', ({ source, forbidden }) => {
    for (const fragment of forbidden) {
      expect(source).not.toContain(fragment);
    }
  });
});
