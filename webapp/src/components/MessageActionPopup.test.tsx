import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { Button } from './ui/button';
import { MessageActionPopup } from './MessageActionPopup';

afterEach(() => {
  cleanup();
});

describe('MessageActionPopup', () => {
  it('renders shadcn message actions in a portal', () => {
    render(
      <div data-testid="app-root">
        <MessageActionPopup
          open
          onClose={() => undefined}
          onAction={() => undefined}
        />
      </div>,
    );

    const popup = screen.getByRole('menu', { name: 'Message actions' });
    expect(popup).toBeInTheDocument();
    expect(document.body).toContainElement(popup);
    expect(popup).toHaveClass('bg-popover', 'rounded-lg', 'shadow-md');

    expect(screen.getByRole('menuitem', { name: 'Reply' })).toBeInTheDocument();
    expect(screen.getByRole('menuitem', { name: 'Reply All' })).toBeInTheDocument();
    expect(screen.getByRole('menuitem', { name: 'Forward' })).toBeInTheDocument();
    expect(screen.getByRole('menuitem', { name: 'Set Aside' })).toBeInTheDocument();
    expect(screen.getByRole('menuitem', { name: 'Reply Later' })).toBeInTheDocument();
    expect(screen.getByRole('menuitem', { name: 'Bubble Up' })).toBeInTheDocument();
    expect(screen.getByText('Move to')).toBeInTheDocument();
    expect(screen.getByRole('menuitem', { name: 'Imbox' })).toBeInTheDocument();
    expect(screen.getByRole('menuitem', { name: 'Feed' })).toBeInTheDocument();
    expect(screen.getByRole('menuitem', { name: 'Paper Trail' })).toBeInTheDocument();
    expect(screen.getByRole('menuitem', { name: 'Add a Note' })).toBeInTheDocument();
    expect(screen.getByRole('menuitem', { name: 'Mark as spam' })).toBeInTheDocument();
    expect(screen.getByRole('menuitem', { name: 'Trash' })).toBeInTheDocument();
  });

  it('calls action callbacks and closes when an action is selected', () => {
    const onAction = vi.fn();
    const onClose = vi.fn();

    render(
      <MessageActionPopup
        open
        onClose={onClose}
        onAction={onAction}
      />,
    );

    fireEvent.click(screen.getByRole('menuitem', { name: 'Reply All' }));
    expect(onAction).toHaveBeenCalledWith('reply-all', undefined);
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it('sends move target as an action payload', () => {
    const onAction = vi.fn();

    render(
      <MessageActionPopup
        open
        onClose={() => undefined}
        onAction={onAction}
      />,
    );

    fireEvent.click(screen.getByRole('menuitem', { name: 'Paper Trail' }));
    expect(onAction).toHaveBeenCalledWith('move-to', 'papertrail');
  });

  it('closes on Escape and when Radix requests close through a trigger', () => {
    const onClose = vi.fn();

    render(
      <div>
        <MessageActionPopup
          open
          onClose={onClose}
          onAction={() => undefined}
          trigger={<Button>Actions</Button>}
        />
      </div>,
    );

    fireEvent.keyDown(document, { key: 'Escape' });
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it('does not render when closed', () => {
    render(
      <MessageActionPopup
        open={false}
        onClose={() => undefined}
        onAction={() => undefined}
      />,
    );

    expect(screen.queryByRole('menu', { name: 'Message actions' })).not.toBeInTheDocument();
  });
});
