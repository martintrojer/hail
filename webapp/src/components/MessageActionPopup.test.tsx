import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { MessageActionPopup } from './MessageActionPopup';

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

describe('MessageActionPopup', () => {
  it('renders message actions in a portal near the anchor', () => {
    render(
      <div data-testid="app-root">
        <MessageActionPopup
          open
          anchorRect={anchorRect()}
          onClose={() => undefined}
          onAction={() => undefined}
        />
      </div>,
    );

    const popup = screen.getByRole('menu', { name: 'Message actions' });
    expect(popup).toBeInTheDocument();
    expect(document.body).toContainElement(popup);
    expect(popup).toHaveStyle({ top: '80px' });

    expect(screen.getByRole('button', { name: 'Reply' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Reply All' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Forward' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Set Aside' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Reply Later' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Bubble Up' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Move to' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Imbox' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Feed' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Paper Trail' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Add a Note' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Mark as spam' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Trash' })).toBeInTheDocument();
  });

  it('calls action callbacks and closes when an action is selected', () => {
    const onAction = vi.fn();
    const onClose = vi.fn();

    render(
      <MessageActionPopup
        open
        anchorRect={anchorRect()}
        onClose={onClose}
        onAction={onAction}
      />,
    );

    fireEvent.click(screen.getByRole('button', { name: 'Reply All' }));
    expect(onAction).toHaveBeenCalledWith('reply-all', undefined);
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it('sends move target as an action payload', () => {
    const onAction = vi.fn();

    render(
      <MessageActionPopup
        open
        anchorRect={anchorRect()}
        onClose={() => undefined}
        onAction={onAction}
      />,
    );

    fireEvent.click(screen.getByRole('button', { name: 'Paper Trail' }));
    expect(onAction).toHaveBeenCalledWith('move-to', 'papertrail');
  });

  it('closes on outside click or Escape', () => {
    const onClose = vi.fn();

    render(
      <div>
        <button type="button">Outside</button>
        <MessageActionPopup
          open
          anchorRect={anchorRect()}
          onClose={onClose}
          onAction={() => undefined}
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
      <MessageActionPopup
        open={false}
        onClose={() => undefined}
        onAction={() => undefined}
      />,
    );

    expect(screen.queryByRole('menu', { name: 'Message actions' })).not.toBeInTheDocument();
  });
});
