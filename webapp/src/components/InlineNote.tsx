export interface InlineNoteProps {
  text: string;
  author: string;
  timestamp: string;
}

export function InlineNote({ text, author, timestamp }: InlineNoteProps) {
  return (
    <article className="rounded-r-lg border-l-4 border-accent-yellow bg-bg-banner p-4">
      <p className="text-xs font-semibold uppercase tracking-wider text-ink-tertiary">
        Note
      </p>
      <p className="mt-2 whitespace-pre-wrap hail-body text-ink-primary">{text}</p>
      <p className="mt-3 text-sm text-ink-tertiary">
        {author} · <time>{timestamp}</time>
      </p>
    </article>
  );
}
