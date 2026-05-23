import { useEffect, useId, useState, type FormEvent } from 'react';
import { HailApiError } from '../api/client';
import { useContact, useContactNoteMutation } from '../api/query';

interface ContactNotePanelProps {
  address: string;
  displayName?: string | null;
}

function contactErrorMessage(error: Error) {
  if (error instanceof HailApiError) {
    if (error.status === 401) {
      return 'Your session expired. Sign in again to view this note.';
    }
    if (error.status === 404) {
      return 'This contact was not found yet.';
    }
    return `Contact note failed with HTTP ${error.status}.`;
  }

  return 'Contact note failed to load. Refresh and try again.';
}

function noteMutationErrorMessage(error: Error) {
  if (error instanceof HailApiError) {
    if (error.status === 400 || error.status === 422) {
      return 'The server rejected this note. Check the markdown and try again.';
    }
    if (error.status === 401) {
      return 'Your session expired. Sign in again before changing this note.';
    }
    return `Contact note update failed with HTTP ${error.status}.`;
  }

  return 'Contact note update failed. Try again.';
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
  const contact = useContact(address);
  const noteMutation = useContactNoteMutation(undefined, {
    onSuccess: (_data, variables) => {
      setSavedMessage(variables.note === null ? 'Note deleted.' : 'Note saved.');
      setMarkdown(variables.note?.markdown ?? '');
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
    <section className="rounded-3xl border border-slate-800 bg-slate-900/70 shadow-lg shadow-slate-950/20">
      <button
        type="button"
        onClick={() => setIsOpen((current) => !current)}
        className="flex w-full items-start justify-between gap-4 p-5 text-left transition hover:bg-slate-900"
        aria-expanded={isOpen}
        aria-controls={textareaId}
      >
        <span className="min-w-0">
          <span className="text-xs font-semibold uppercase tracking-[0.3em] text-sky-300">
            Contact note
          </span>
          <span className="mt-2 block truncate text-base font-semibold text-slate-100">
            {title}
          </span>
          <span className="mt-1 block truncate text-sm text-slate-500">
            {address}
          </span>
          {!isOpen ? (
            <span className="mt-3 line-clamp-2 block text-sm leading-6 text-slate-400">
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
        <span className="shrink-0 rounded-full border border-slate-700 bg-slate-950 px-3 py-1 text-xs font-semibold text-slate-300">
          {isOpen ? 'Hide' : hasSavedNote ? 'Edit' : 'Add'}
        </span>
      </button>

      {isOpen ? (
        <div className="border-t border-slate-800 p-5">
          {contact.isPending ? (
            <div className="space-y-3" aria-label="Loading contact note">
              <div className="h-4 w-1/3 animate-pulse rounded bg-slate-800" />
              <div className="h-28 animate-pulse rounded-xl bg-slate-800" />
            </div>
          ) : contact.isError ? (
            <p
              role="alert"
              className="rounded-xl border border-red-400/30 bg-red-400/10 p-4 text-sm text-red-100"
            >
              {contactErrorMessage(contact.error)}
            </p>
          ) : (
            <form onSubmit={save} className="space-y-3">
              <label
                htmlFor={textareaId}
                className="block text-sm font-medium text-slate-200"
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
                className="w-full resize-y rounded-xl border border-slate-700 bg-slate-950 px-3 py-2 text-sm leading-6 text-slate-100 outline-none ring-sky-400 transition placeholder:text-slate-600 focus:border-sky-400 focus:ring-2 disabled:cursor-not-allowed disabled:opacity-60"
              />

              <div className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
                <div className="text-xs text-slate-500">
                  {updatedAt ? (
                    <span>Last updated {updatedAt}</span>
                  ) : (
                    <span>No saved note.</span>
                  )}
                  {isDirty ? (
                    <span className="ml-2 text-amber-200">Unsaved changes</span>
                  ) : null}
                </div>
                <div className="flex gap-2">
                  <button
                    type="button"
                    onClick={deleteNote}
                    disabled={isBusy || (!hasSavedNote && !hasDraft)}
                    className="rounded-lg border border-slate-700 px-3 py-2 text-sm font-semibold text-slate-100 transition hover:border-red-400 hover:text-red-100 disabled:cursor-not-allowed disabled:opacity-60"
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
                    className="rounded-lg bg-sky-400 px-4 py-2 text-sm font-semibold text-slate-950 transition hover:bg-sky-300 disabled:cursor-not-allowed disabled:opacity-60"
                  >
                    {noteMutation.isPending ? 'Saving…' : 'Save'}
                  </button>
                </div>
              </div>

              {savedMessage ? (
                <p className="text-sm text-emerald-200" role="status">
                  {savedMessage}
                </p>
              ) : null}
              {noteMutation.isError ? (
                <p role="alert" className="text-sm text-red-200">
                  {noteMutationErrorMessage(noteMutation.error)}
                </p>
              ) : null}
            </form>
          )}
        </div>
      ) : null}
    </section>
  );
}
