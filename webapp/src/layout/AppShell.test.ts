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
    classes: [
      'max-w-full',
      'md:max-w-[min(100%,calc(100vw-var(--sidebar-width-icon)-3rem))]',
      'lg:max-w-[min(100%,calc(100vw-var(--sidebar-width-icon)-4rem))]',
    ],
  },
  {
    layout: 'split',
    classes: [
      'max-w-full',
      'md:max-w-[min(100%,calc(100vw-var(--sidebar-width-icon)-3rem))]',
      'lg:max-w-[min(100%,calc(100vw-var(--sidebar-width-icon)-4rem))]',
      'xl:max-w-7xl',
    ],
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

  it('keeps list and split layouts responsive to the collapsed sidebar instead of hard-coding page widths', () => {
    expect(appShellContentWidthClass('list')).toContain('calc(100vw-var(--sidebar-width-icon)-3rem)');
    expect(appShellContentWidthClass('split')).toContain('calc(100vw-var(--sidebar-width-icon)-3rem)');
  });
});
