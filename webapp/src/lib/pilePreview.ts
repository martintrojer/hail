import type { PileItem } from '../api/client';

export interface PilePreview {
  sender: string;
  subject: string;
  snippet: string | null;
}

function textFrom(value: unknown): string | null {
  if (value === null || value === undefined) {
    return null;
  }

  if (typeof value === 'string') {
    return value.trim() || null;
  }

  if (typeof value === 'number' || typeof value === 'boolean') {
    return String(value);
  }

  if (Array.isArray(value)) {
    return value.map(textFrom).filter(Boolean).join(' · ') || null;
  }

  if (typeof value === 'object') {
    const record = value as Record<string, unknown>;
    const candidates = [
      record.name,
      record.email,
      record.sender,
      record.from,
      record.subject,
      record.title,
      record.preview,
      record.snippet,
    ];

    for (const candidate of candidates) {
      const text = textFrom(candidate);
      if (text) {
        return text;
      }
    }
  }

  return null;
}

function fieldText(record: Record<string, unknown>, keys: string[]): string | null {
  for (const key of keys) {
    const text = textFrom(record[key]);
    if (text) {
      return text;
    }
  }

  return null;
}

export function pilePreview(item: PileItem): PilePreview {
  const preview = item.preview;
  const fallbackSubject = item.thread_id;

  if (preview && typeof preview === 'object' && !Array.isArray(preview)) {
    const record = preview as Record<string, unknown>;
    return {
      sender: fieldText(record, ['from', 'sender', 'name', 'email', 'author']) ?? 'Saved thread',
      subject: fieldText(record, ['subject', 'title']) ?? fallbackSubject,
      snippet: fieldText(record, ['snippet', 'preview', 'body', 'excerpt']),
    };
  }

  const text = textFrom(preview);
  return {
    sender: 'Saved thread',
    subject: text ?? fallbackSubject,
    snippet: text && text !== fallbackSubject ? null : null,
  };
}

export function formatPileDate(value: string) {
  const date = new Date(value);
  if (Number.isNaN(date.valueOf())) {
    return value;
  }

  const now = new Date();
  const sameYear = date.getFullYear() === now.getFullYear();

  return new Intl.DateTimeFormat(undefined, {
    month: 'short',
    day: 'numeric',
    ...(sameYear ? {} : { year: 'numeric' }),
    hour: 'numeric',
    minute: '2-digit',
  }).format(date);
}
