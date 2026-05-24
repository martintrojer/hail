import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { ScreenerRoutingDropdown } from './ScreenerRoutingDropdown';

function anchorRect(overrides: Partial<DOMRect> = {}): DOMRect {
  return {
    x: 100,
    y: 40,
    width: 72,
    height: 32,
    top: 40,
    right: 172,
    bottom: 72,
    left: 100,
    toJSON: () => ({}),
    ...overrides,
  };
}

afterEach(() => {
  cleanup();
});

describe('ScreenerRoutingDropdown', () => {
  it('renders routing destinations in a portal with Imbox highlighted', () => {
    render(
      <div data-testid="app-root">
        <ScreenerRoutingDropdown
          open
          anchorRect={anchorRect()}
          onClose={() => undefined}
          onSelect={() => undefined}
        />
      </div>,
    );

    const dropdown = screen.getByRole('menu', {
      name: 'Screener routing destinations',
    });
    expect(dropdown).toBeInTheDocument();
    expect(document.body).toContainElement(dropdown);
    expect(dropdown).toHaveStyle({ top: '80px', left: '100px' });
    expect(dropdown).toHaveClass(
      'bg-bg-surface',
      'border-border-menu',
      'rounded-lg',
      'shadow-md',
    );

    const imbox = screen.getByRole('menuitem', { name: 'The Imbox' });
    expect(imbox).toHaveClass('bg-bg-selected');
    expect(screen.getByRole('menuitem', { name: 'The Feed' })).toBeInTheDocument();
    expect(screen.getByRole('menuitem', { name: 'Paper Trail' })).toBeInTheDocument();
  });

  it('selects a destination and closes', () => {
    const onSelect = vi.fn();
    const onClose = vi.fn();

    render(
      <ScreenerRoutingDropdown
        open
        anchorRect={anchorRect()}
        onClose={onClose}
        onSelect={onSelect}
      />,
    );

    fireEvent.click(screen.getByRole('menuitem', { name: 'The Feed' }));

    expect(onSelect).toHaveBeenCalledWith('feed');
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it('closes on outside click or Escape', () => {
    const onClose = vi.fn();

    render(
      <div>
        <button type="button">Outside</button>
        <ScreenerRoutingDropdown
          open
          anchorRect={anchorRect()}
          onClose={onClose}
          onSelect={() => undefined}
        />
      </div>,
    );

    fireEvent.mouseDown(screen.getByRole('button', { name: 'Outside' }));
    expect(onClose).toHaveBeenCalledTimes(1);

    fireEvent.keyDown(document, { key: 'Escape' });
    expect(onClose).toHaveBeenCalledTimes(2);
  });

  it('does not render when closed', () => {
    render(
      <ScreenerRoutingDropdown
        open={false}
        onClose={() => undefined}
        onSelect={() => undefined}
      />,
    );

    expect(
      screen.queryByRole('menu', { name: 'Screener routing destinations' }),
    ).not.toBeInTheDocument();
  });
});
