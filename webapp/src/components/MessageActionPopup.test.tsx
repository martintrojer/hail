import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { useState } from 'react';
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

  it('activates the first action with ArrowDown and Enter from the trigger', async () => {
    const onAction = vi.fn();

    function Harness() {
      const [open, setOpen] = useState(false);
      return (
        <MessageActionPopup
          open={open}
          onClose={() => setOpen(false)}
          onOpenChange={setOpen}
          onAction={onAction}
          trigger={<Button>Actions</Button>}
        />
      );
    }

    render(<Harness />);

    const trigger = screen.getByRole('button', { name: 'Actions' });
    trigger.focus();
    fireEvent.keyDown(trigger, { key: 'ArrowDown', code: 'ArrowDown' });
    expect(await screen.findByRole('menu')).toHaveAttribute('aria-label', 'Message actions');
    fireEvent.keyDown(document.activeElement ?? screen.getByRole('menu'), {
      key: 'Enter',
      code: 'Enter',
    });

    await waitFor(() => expect(onAction).toHaveBeenCalledWith('reply', undefined));
    await waitFor(() =>
      expect(screen.queryByRole('menu')).not.toBeInTheDocument(),
    );
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
