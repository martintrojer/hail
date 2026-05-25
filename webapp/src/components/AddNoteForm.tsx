import { useState, type FormEvent } from 'react';

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
    <form className="space-y-3" onSubmit={handleSubmit} aria-label="Add note">
      <textarea
        value={text}
        onChange={(event) => setText(event.target.value)}
        className="min-h-28 w-full resize-y rounded-lg border border-border-hairline bg-bg-surface p-3 hail-body text-ink-primary outline-none placeholder:text-ink-tertiary focus:border-accent-blue"
        placeholder="Add a note…"
        aria-label="Note text"
      />
      <div className="flex items-center gap-3">
        <button
          type="submit"
          disabled={!trimmedText}
          className="rounded-lg bg-accent-blue px-3 py-1.5 text-sm font-semibold text-white focus-ring outline-none hover:bg-accent-blue-hover disabled:cursor-not-allowed disabled:opacity-60"
        >
          Save
        </button>
        <button
          type="button"
          onClick={onCancel}
          className="text-sm font-medium text-ink-tertiary focus-ring outline-none hover:text-ink-primary focus-visible:rounded-md"
        >
          Cancel
        </button>
      </div>
    </form>
  );
}
