import { describe, expect, it } from 'vitest';
import {
  appShellContentWidthClass,
  type AppShellContentLayout,
} from './AppShell';

const expectedLayouts: Array<{
  layout: AppShellContentLayout;
  classes: string[];
}> = [
  {
    layout: 'list',
    classes: ['max-w-none'],
  },
  {
    layout: 'split',
    classes: ['max-w-none', 'xl:max-w-7xl'],
  },
  {
    layout: 'reading',
    classes: ['max-w-3xl', 'lg:max-w-4xl', 'xl:max-w-5xl'],
  },
  {
    layout: 'composer',
    classes: ['max-w-3xl', 'lg:max-w-4xl', 'xl:max-w-5xl'],
  },
  {
    layout: 'wide',
    classes: ['max-w-6xl'],
  },
];

describe('AppShell content containers', () => {
  it.each(expectedLayouts)('resolves the $layout layout variant from one central map', ({ layout, classes }) => {
    const resolved = appShellContentWidthClass(layout);

    for (const className of classes) {
      expect(resolved).toContain(className);
    }
  });

  it('keeps list and split layouts inside the flex-owned AppShell inset without viewport math', () => {
    expect(appShellContentWidthClass('list')).toBe('max-w-none');
    expect(appShellContentWidthClass('split')).toBe('max-w-none xl:max-w-7xl');
  });

  it('does not use viewport width calculations that can double-count the sidebar and pin horizontal scrolling', () => {
    for (const layout of ['list', 'split'] satisfies AppShellContentLayout[]) {
      const resolved = appShellContentWidthClass(layout);

      expect(resolved).not.toContain('vw');
      expect(resolved).not.toContain('calc(');
    }
  });
});
