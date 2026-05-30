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

    openDropdown();

    const dropdown = screen.getByRole('menu');
    expect(dropdown).toBeInTheDocument();
    expect(document.body).toContainElement(dropdown);

    const imbox = screen.getByRole('menuitemradio', { name: 'The Imbox' });
    expect(imbox).toHaveAttribute('aria-checked', 'true');
    expect(screen.getByRole('menuitemradio', { name: 'The Feed' })).toHaveAttribute(
      'aria-checked',
      'false',
    );
    expect(screen.getByRole('menuitemradio', { name: 'Paper Trail' })).toHaveAttribute(
      'aria-checked',
      'false',
    );
  });

  it('selects a destination from the menu', async () => {
    const onSelect = renderRoutingDropdown();

    openDropdown();
    fireEvent.click(screen.getByRole('menuitemradio', { name: 'The Feed' }));

    expect(onSelect).toHaveBeenCalledWith('feed');
    await waitFor(() =>
      expect(screen.queryByRole('menu')).not.toBeInTheDocument(),
    );
  });

  it('selects Feed with ArrowDown and Enter from the open menu', async () => {
    const onSelect = renderRoutingDropdown();

    openDropdown();
    await screen.findByRole('menu');
    const feed = screen.getByRole('menuitemradio', { name: 'The Feed' });
    feed.focus();
    fireEvent.keyDown(feed, { key: 'ArrowDown', code: 'ArrowDown' });
    fireEvent.keyDown(feed, {
      key: 'Enter',
      code: 'Enter',
    });

    await waitFor(() => expect(onSelect).toHaveBeenCalledWith('feed'));
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
