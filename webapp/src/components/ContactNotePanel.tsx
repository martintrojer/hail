import { useEffect, useId, useState, type FormEvent } from 'react';
import { useContact, useContactNoteMutation } from '../api/query';
import { contactErrorMessage, contactNoteMutationErrorMessage } from '../lib/errorMessages';
import { useUndoToast } from './UndoToastProvider';

interface ContactNotePanelProps {
  address: string;
  displayName?: string | null;
}

function formatUpdatedAt(value: string | null | undefined) {
  if (!value) {
    return null;
  }

  const date = new Date(value);
  if (Number.isNaN(date.valueOf())) {
    return value;
  }

  return new Intl.DateTimeFormat(undefined, {
    dateStyle: 'medium',
    timeStyle: 'short',
  }).format(date);
}

export function ContactNotePanel({
  address,
  displayName,
}: ContactNotePanelProps) {
  const textareaId = useId();
  const [isOpen, setIsOpen] = useState(false);
  const [markdown, setMarkdown] = useState('');
  const [savedMessage, setSavedMessage] = useState<string | null>(null);
  const { showToast } = useUndoToast();
  const contact = useContact(address);
  const noteMutation = useContactNoteMutation(undefined, {
    onSuccess: (_data, variables) => {
      const deleted = variables.note === null;
      setSavedMessage(deleted ? null : 'Note saved.');
      setMarkdown(variables.note?.markdown ?? '');
      if (deleted) {
        showToast({ message: 'Note deleted.' });
      }
    },
  });

  const serverMarkdown = contact.data?.note?.markdown ?? '';
  const isBusy = contact.isPending || noteMutation.isPending;
  const hasSavedNote =
    contact.data?.note !== null && contact.data?.note !== undefined;
  const hasNoteText = serverMarkdown.trim().length > 0;
  const hasDraft = markdown.trim().length > 0;
  const isDirty = markdown !== serverMarkdown;
  const updatedAt = formatUpdatedAt(contact.data?.note?.updated_at);
  const title = displayName?.trim() || address;

  useEffect(() => {
    setMarkdown(serverMarkdown);
  }, [address, contact.dataUpdatedAt, serverMarkdown]);

  function save(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    setSavedMessage(null);
    noteMutation.mutate({ address, note: { markdown } });
  }

  function deleteNote() {
    setSavedMessage(null);

    if (hasSavedNote) {
      noteMutation.mutate({ address, note: null });
    } else {
      setMarkdown('');
      setSavedMessage('Draft cleared.');
    }
  }

  return (
    <section className="rounded-lg border border-hairline bg-surface shadow-md">
      <button
        type="button"
        onClick={() => setIsOpen((current) => !current)}
        className="flex w-full items-start justify-between gap-4 p-5 text-left transition hover:bg-hover"
        aria-expanded={isOpen}
        aria-controls={textareaId}
      >
        <span className="min-w-0">
          <span className="text-xs font-semibold uppercase tracking-[0.3em] text-accent-blue">
            Contact note
          </span>
          <span className="mt-2 block truncate text-base font-semibold text-ink-primary">
            {title}
          </span>
          <span className="mt-1 block truncate text-sm text-ink-primary0">
            {address}
          </span>
          {!isOpen ? (
            <span className="mt-3 line-clamp-2 block text-sm leading-6 text-ink-secondary">
              {contact.isPending
                ? 'Loading note…'
                : contact.isError
                  ? contactErrorMessage(contact.error)
                  : hasNoteText
                    ? serverMarkdown
                    : 'No note yet. Add private markdown context for this sender.'}
            </span>
          ) : null}
        </span>
        <span className="shrink-0 rounded-full border border-hairline bg-page px-3 py-1 text-xs font-semibold text-ink-secondary">
          {isOpen ? 'Hide' : hasSavedNote ? 'Edit' : 'Add'}
        </span>
      </button>

      {isOpen ? (
        <div className="border-t border-hairline p-5">
          {contact.isPending ? (
            <div className="space-y-3" aria-label="Loading contact note">
              <div className="h-4 w-1/3 animate-pulse rounded bg-hover" />
              <div className="h-28 animate-pulse rounded-lg bg-hover" />
            </div>
          ) : contact.isError ? (
            <p
              role="alert"
              className="rounded-lg border border-accent-red/30 bg-accent-red/10 p-4 text-sm text-accent-red"
            >
              {contactErrorMessage(contact.error)}
            </p>
          ) : (
            <form onSubmit={save} className="space-y-3">
              <label
                htmlFor={textareaId}
                className="block text-sm font-medium text-ink-primary"
              >
                Markdown note
              </label>
              <textarea
                id={textareaId}
                value={markdown}
                onChange={(event) => {
                  setMarkdown(event.target.value);
                  setSavedMessage(null);
                }}
                disabled={noteMutation.isPending}
                rows={6}
                placeholder="Add reminders, context, preferences, or links for this contact. Markdown is stored as plain text."
                className="w-full resize-y rounded-lg border border-hairline bg-page px-3 py-2 text-sm leading-6 text-ink-primary outline-none ring-accent-blue transition placeholder:text-ink-tertiary focus:border-accent-blue focus:ring-2 disabled:cursor-not-allowed disabled:opacity-60"
              />

              <div className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
                <div className="text-xs text-ink-primary0">
                  {updatedAt ? (
                    <span>Last updated {updatedAt}</span>
                  ) : (
                    <span>No saved note.</span>
                  )}
                  {isDirty ? (
                    <span className="ml-2 text-accent-yellow">Unsaved changes</span>
                  ) : null}
                </div>
                <div className="flex gap-2">
                  <button
                    type="button"
                    onClick={deleteNote}
                    disabled={isBusy || (!hasSavedNote && !hasDraft)}
                    className="rounded-lg border border-hairline px-3 py-2 text-sm font-semibold text-ink-primary transition hover:border-accent-red hover:text-accent-red disabled:cursor-not-allowed disabled:opacity-60"
                  >
                    {noteMutation.isPending
                      ? 'Working…'
                      : hasSavedNote
                        ? 'Delete'
                        : 'Clear'}
                  </button>
                  <button
                    type="submit"
                    disabled={isBusy || !isDirty}
                    className="rounded-lg bg-accent-blue px-4 py-2 text-sm font-semibold text-white transition hover:bg-accent-blue-hover disabled:cursor-not-allowed disabled:opacity-60"
                  >
                    {noteMutation.isPending ? 'Saving…' : 'Save'}
                  </button>
                </div>
              </div>

              {savedMessage ? (
                <p className="text-sm text-accent-blue" role="status">
                  {savedMessage}
                </p>
              ) : null}
              {noteMutation.isError ? (
                <p role="alert" className="text-sm text-accent-red">
                  {contactNoteMutationErrorMessage(noteMutation.error)}
                </p>
              ) : null}
            </form>
          )}
        </div>
      ) : null}
    </section>
  );
}
