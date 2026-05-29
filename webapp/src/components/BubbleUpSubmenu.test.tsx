import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { BubbleUpSubmenu } from './BubbleUpSubmenu';

afterEach(() => {
  cleanup();
});

describe('BubbleUpSubmenu', () => {
  it('renders preset bubble up time options in a shadcn portal menu', () => {
    render(
      <div data-testid="app-root">
        <BubbleUpSubmenu
          open
          onClose={() => undefined}
          onSelect={() => undefined}
        />
      </div>,
    );

    const submenu = screen.getByRole('menu', { name: 'Bubble up time options' });
    expect(submenu).toBeInTheDocument();
    expect(document.body).toContainElement(submenu);
    expect(submenu).toHaveClass('bg-popover', 'rounded-lg', 'shadow-md');

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
        onClose={onClose}
        onSelect={onSelect}
      />,
    );

    fireEvent.click(screen.getByRole('menuitem', { name: 'Tomorrow morning' }));

    expect(onSelect).toHaveBeenCalledWith('Tomorrow morning');
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it('selects an option with ArrowDown and Enter, then closes on Escape', async () => {
    const onSelect = vi.fn();
    const onClose = vi.fn();

    render(
      <BubbleUpSubmenu
        open
        onClose={onClose}
        onSelect={onSelect}
      />,
    );

    const menu = screen.getByRole('menu', { name: 'Bubble up time options' });
    fireEvent.keyDown(menu, { key: 'ArrowDown', code: 'ArrowDown' });
    fireEvent.keyDown(document.activeElement ?? menu, { key: 'Enter', code: 'Enter' });

    await waitFor(() => expect(onSelect).toHaveBeenCalledWith('Later today'));
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it('closes on Escape', () => {
    const onClose = vi.fn();

    render(
      <BubbleUpSubmenu
        open
        onClose={onClose}
        onSelect={() => undefined}
      />,
    );

    fireEvent.keyDown(document, { key: 'Escape' });
    expect(onClose).toHaveBeenCalledTimes(1);
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
