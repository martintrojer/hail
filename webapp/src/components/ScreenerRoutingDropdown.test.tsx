import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { Button } from './ui/button';
import { ScreenerRoutingDropdown } from './ScreenerRoutingDropdown';

afterEach(() => {
  cleanup();
});

function renderRoutingDropdown(onSelect = vi.fn()) {
  render(
    <ScreenerRoutingDropdown onSelect={onSelect}>
      <Button type="button">Approve</Button>
    </ScreenerRoutingDropdown>,
  );

  return onSelect;
}

function openDropdown() {
  fireEvent.pointerDown(screen.getByRole('button', { name: 'Approve' }), {
    ctrlKey: false,
    button: 0,
  });
}

describe('ScreenerRoutingDropdown', () => {
  it('opens routing destinations from its trigger with Imbox highlighted', () => {
    renderRoutingDropdown();

    expect(screen.queryByRole('menu')).not.toBeInTheDocument();

    openDropdown();

    const dropdown = screen.getByRole('menu');
    expect(dropdown).toBeInTheDocument();
    expect(document.body).toContainElement(dropdown);

    const imbox = screen.getByRole('menuitem', { name: 'The Imbox' });
    expect(imbox).toHaveClass('bg-muted');
    expect(imbox).toHaveAttribute('aria-checked', 'true');
    expect(screen.getByRole('menuitem', { name: 'The Feed' })).toHaveAttribute(
      'aria-checked',
      'false',
    );
    expect(screen.getByRole('menuitem', { name: 'Paper Trail' })).toBeInTheDocument();
  });

  it('selects a destination from the menu', () => {
    const onSelect = renderRoutingDropdown();

    openDropdown();
    fireEvent.click(screen.getByRole('menuitem', { name: 'The Feed' }));

    expect(onSelect).toHaveBeenCalledWith('feed');
  });

  it('activates the initially focused item with keyboard ArrowDown and Enter', async () => {
    const onSelect = renderRoutingDropdown();
    const trigger = screen.getByRole('button', { name: 'Approve' });

    trigger.focus();
    fireEvent.keyDown(trigger, { key: 'ArrowDown', code: 'ArrowDown' });
    fireEvent.keyDown(document.activeElement ?? screen.getByRole('menu'), {
      key: 'Enter',
      code: 'Enter',
    });

    await waitFor(() => expect(onSelect).toHaveBeenCalledWith('imbox'));
  });

  it('closes on Escape', async () => {
    renderRoutingDropdown();

    openDropdown();
    expect(screen.getByRole('menu')).toBeInTheDocument();

    fireEvent.keyDown(document.activeElement ?? screen.getByRole('menu'), {
      key: 'Escape',
    });

    await waitFor(() =>
      expect(
        screen.queryByRole('menu'),
      ).not.toBeInTheDocument(),
    );
  });
});
