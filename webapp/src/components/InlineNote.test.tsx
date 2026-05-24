import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { AddNoteForm } from './AddNoteForm';
import { InlineNote } from './InlineNote';

afterEach(() => {
  cleanup();
});

describe('InlineNote', () => {
  it('renders note content and metadata inline', () => {
    render(
      <InlineNote
        text="Remember to ask about the invoice before replying."
        author="Mina"
        timestamp="Today at 10:15"
      />,
    );

    expect(screen.getByText('Note')).toBeInTheDocument();
    expect(screen.getByText('Remember to ask about the invoice before replying.')).toBeInTheDocument();
    expect(screen.getByText(/Mina/)).toHaveTextContent('Mina · Today at 10:15');
  });

  it('uses the warm inline note treatment', () => {
    render(<InlineNote text="Private thread context" author="Ari" timestamp="Yesterday" />);

    expect(screen.getByRole('article')).toHaveClass(
      'border-l-4',
      'border-accent-yellow',
      'bg-bg-banner',
      'rounded-r-lg',
      'p-4',
    );
  });
});

describe('AddNoteForm', () => {
  it('saves trimmed note text', () => {
    const onSave = vi.fn();

    render(<AddNoteForm onSave={onSave} onCancel={() => undefined} />);

    fireEvent.change(screen.getByLabelText('Note text'), {
      target: { value: '  Follow up after lunch.  ' },
    });
    fireEvent.click(screen.getByRole('button', { name: 'Save' }));

    expect(onSave).toHaveBeenCalledWith('Follow up after lunch.');
  });

  it('cancels without saving', () => {
    const onCancel = vi.fn();
    const onSave = vi.fn();

    render(<AddNoteForm onSave={onSave} onCancel={onCancel} />);

    fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));

    expect(onCancel).toHaveBeenCalledTimes(1);
    expect(onSave).not.toHaveBeenCalled();
  });

  it('does not save blank notes', () => {
    const onSave = vi.fn();

    render(<AddNoteForm onSave={onSave} onCancel={() => undefined} />);

    fireEvent.change(screen.getByLabelText('Note text'), {
      target: { value: '   ' },
    });
    fireEvent.submit(screen.getByRole('form', { name: 'Add note' }));

    expect(onSave).not.toHaveBeenCalled();
  });
});
