import { useEffect, useId, useState, type FormEvent } from 'react';
import { useContact, useContactNoteMutation } from '../api/query';
import { contactErrorMessage, contactNoteMutationErrorMessage } from '../lib/errorMessages';
import { useUndoToast } from './UndoToastProvider';
import { Alert, AlertDescription } from './ui/alert';
import { Badge } from './ui/badge';
import { Button } from './ui/button';
import {
  Card,
  CardAction,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from './ui/card';
import { Skeleton } from './ui/skeleton';
import { Textarea } from './ui/textarea';

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
    <Card size="sm">
      <button
        type="button"
        onClick={() => setIsOpen((current) => !current)}
        className="w-full text-left outline-none focus-visible:ring-3 focus-visible:ring-ring/50"
        aria-expanded={isOpen}
        aria-controls={textareaId}
      >
        <CardHeader className="transition hover:bg-muted/50">
          <CardTitle className="truncate">{title}</CardTitle>
          <CardDescription className="truncate">{address}</CardDescription>
          <CardAction>
            <Badge variant="secondary">{isOpen ? 'Hide' : hasSavedNote ? 'Edit' : 'Add'}</Badge>
          </CardAction>
          {!isOpen ? (
            <CardDescription className="line-clamp-2">
              {contact.isPending
                ? 'Loading note…'
                : contact.isError
                  ? contactErrorMessage(contact.error)
                  : hasNoteText
                    ? serverMarkdown
                    : 'No note yet. Add private markdown context for this sender.'}
            </CardDescription>
          ) : null}
        </CardHeader>
      </button>

      {isOpen ? (
        <CardContent>
          {contact.isPending ? (
            <div className="flex flex-col gap-3" aria-label="Loading contact note">
              <Skeleton className="h-4 w-1/3" />
              <Skeleton className="h-28" />
            </div>
          ) : contact.isError ? (
            <Alert variant="destructive">
              <AlertDescription>{contactErrorMessage(contact.error)}</AlertDescription>
            </Alert>
          ) : (
            <form onSubmit={save} className="flex flex-col gap-3">
              <label
                htmlFor={textareaId}
                className="text-sm font-medium text-foreground"
              >
                Markdown note
              </label>
              <Textarea
                id={textareaId}
                value={markdown}
                onChange={(event) => {
                  setMarkdown(event.target.value);
                  setSavedMessage(null);
                }}
                disabled={noteMutation.isPending}
                rows={6}
                placeholder="Add reminders, context, preferences, or links for this contact. Markdown is stored as plain text."
              />

              <div className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
                <div className="text-xs text-muted-foreground">
                  {updatedAt ? (
                    <span>Last updated {updatedAt}</span>
                  ) : (
                    <span>No saved note.</span>
                  )}
                  {isDirty ? (
                    <span className="ml-2 text-foreground">Unsaved changes</span>
                  ) : null}
                </div>
                <div className="flex gap-2">
                  <Button
                    type="button"
                    variant="outline"
                    size="sm"
                    onClick={deleteNote}
                    disabled={isBusy || (!hasSavedNote && !hasDraft)}
                  >
                    {noteMutation.isPending
                      ? 'Working…'
                      : hasSavedNote
                        ? 'Delete'
                        : 'Clear'}
                  </Button>
                  <Button
                    type="submit"
                    size="sm"
                    disabled={isBusy || !isDirty}
                  >
                    {noteMutation.isPending ? 'Saving…' : 'Save'}
                  </Button>
                </div>
              </div>

              {savedMessage ? (
                <p className="text-sm text-muted-foreground" role="status">
                  {savedMessage}
                </p>
              ) : null}
              {noteMutation.isError ? (
                <Alert variant="destructive">
                  <AlertDescription>
                    {contactNoteMutationErrorMessage(noteMutation.error)}
                  </AlertDescription>
                </Alert>
              ) : null}
            </form>
          )}
        </CardContent>
      ) : null}
    </Card>
  );
}
