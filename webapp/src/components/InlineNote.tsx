import { StickyNote } from 'lucide-react';
import { Card, CardContent, CardHeader, CardTitle } from './ui/card';

export interface InlineNoteProps {
  text: string;
  author: string;
  timestamp: string;
}

export function InlineNote({ text, author, timestamp }: InlineNoteProps) {
  return (
    <Card size="sm" role="article" className="rounded-r-lg border-l-4 border-l-primary bg-muted/40">
      <CardHeader>
        <CardTitle className="flex items-center gap-2 text-xs uppercase tracking-wider text-muted-foreground">
          <StickyNote aria-hidden="true" />
          Note
        </CardTitle>
      </CardHeader>
      <CardContent>
        <p className="whitespace-pre-wrap hail-body text-foreground">{text}</p>
        <p className="mt-3 text-sm text-muted-foreground">
          {author} · <time>{timestamp}</time>
        </p>
      </CardContent>
    </Card>
  );
}
