import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { BubbleUpSubmenu } from './BubbleUpSubmenu';

function anchorRect(overrides: Partial<DOMRect> = {}): DOMRect {
  return {
    x: 100,
    y: 40,
    width: 32,
    height: 32,
    top: 40,
    right: 132,
    bottom: 72,
    left: 100,
    toJSON: () => ({}),
    ...overrides,
  };
}

afterEach(() => {
  cleanup();
});

describe('BubbleUpSubmenu', () => {
  it('renders preset bubble up time options in a portal', () => {
    render(
      <div data-testid="app-root">
        <BubbleUpSubmenu
          open
          anchorRect={anchorRect()}
          onClose={() => undefined}
          onSelect={() => undefined}
        />
      </div>,
    );

    const submenu = screen.getByRole('menu', { name: 'Bubble up time options' });
    expect(submenu).toBeInTheDocument();
    expect(document.body).toContainElement(submenu);
    expect(submenu).toHaveStyle({ top: '40px', left: '140px' });
    expect(submenu).toHaveClass(
      'bg-bg-surface',
      'border-border-menu',
      'rounded-lg',
      'shadow-md',
    );

    expect(screen.getByRole('menuitem', { name: 'Later today' })).toBeInTheDocument();
    expect(screen.getByRole('menuitem', { name: 'Tomorrow morning' })).toBeInTheDocument();
    expect(screen.getByRole('menuitem', { name: 'This weekend' })).toBeInTheDocument();
    expect(screen.getByRole('menuitem', { name: 'Next week' })).toBeInTheDocument();
    expect(screen.getByRole('menuitem', { name: 'Pick a date…' })).toBeInTheDocument();
  });

  it('selects an option and closes', () => {
    const onSelect = vi.fn();
    const onClose = vi.fn();

    render(
      <BubbleUpSubmenu
        open
        anchorRect={anchorRect()}
        onClose={onClose}
        onSelect={onSelect}
      />,
    );

    fireEvent.click(screen.getByRole('menuitem', { name: 'Tomorrow morning' }));

    expect(onSelect).toHaveBeenCalledWith('Tomorrow morning');
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it('closes on outside click or Escape', () => {
    const onClose = vi.fn();

    render(
      <div>
        <button type="button">Outside</button>
        <BubbleUpSubmenu
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
      <BubbleUpSubmenu
        open={false}
        onClose={() => undefined}
        onSelect={() => undefined}
      />,
    );

    expect(screen.queryByRole('menu', { name: 'Bubble up time options' })).not.toBeInTheDocument();
  });
});
