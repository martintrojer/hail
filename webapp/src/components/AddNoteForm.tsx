import { useState, type FormEvent } from 'react';
import { Button } from './ui/button';
import { Textarea } from './ui/textarea';

export interface AddNoteFormProps {
  onSave: (text: string) => void;
  onCancel: () => void;
}

export function AddNoteForm({ onSave, onCancel }: AddNoteFormProps) {
  const [text, setText] = useState('');
  const trimmedText = text.trim();

  function handleSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (!trimmedText) {
      return;
    }

    onSave(trimmedText);
  }

  return (
    <form className="flex flex-col gap-3" onSubmit={handleSubmit} aria-label="Add note">
      <Textarea
        value={text}
        onChange={(event) => setText(event.target.value)}
        className="min-h-28 resize-y"
        autoFocus
        placeholder="Add a note…"
        aria-label="Note text"
      />
      <div className="flex items-center gap-2">
        <Button type="submit" size="sm" disabled={!trimmedText}>
          Save
        </Button>
        <Button type="button" size="sm" variant="ghost" onClick={onCancel}>
          Cancel
        </Button>
      </div>
    </form>
  );
}
