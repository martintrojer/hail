import { forwardRef, useEffect, useImperativeHandle, useState } from 'react';
import Link from '@tiptap/extension-link';
import Placeholder from '@tiptap/extension-placeholder';
import Underline from '@tiptap/extension-underline';
import { EditorContent, useEditor, type Editor } from '@tiptap/react';
import StarterKit from '@tiptap/starter-kit';
import {
  Bold,
  Code,
  Heading2,
  Heading3,
  Italic,
  LinkIcon,
  List,
  ListOrdered,
  Minus,
  Quote,
  Redo2,
  RemoveFormatting,
  Strikethrough,
  Underline as UnderlineIcon,
  Undo2,
} from 'lucide-react';
import { Button } from '../ui/button';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from '../ui/dropdown-menu';
import { Input } from '../ui/input';
import { Popover, PopoverContent, PopoverTrigger } from '../ui/popover';
import { Separator } from '../ui/separator';
import { ToggleGroup, ToggleGroupItem } from '../ui/toggle-group';

export interface RichTextEditorHandle {
  focus: (position?: 'start' | 'end') => void;
}

interface RichTextEditorProps {
  value: string;
  onChange: (value: string) => void;
  onReady?: (editor: Editor) => void;
  placeholder?: string;
  autoFocus?: boolean;
  id?: string;
  'aria-label'?: string;
}

function normalizedHtml(value: string) {
  return value.trim() || '<p></p>';
}

export const RichTextEditor = forwardRef<RichTextEditorHandle, RichTextEditorProps>(function RichTextEditor({
  value,
  onChange,
  onReady,
  placeholder = 'Write your email…',
  autoFocus = false,
  id = 'compose-body',
  'aria-label': ariaLabel = 'Body',
}, ref) {
  const [, forceToolbarUpdate] = useState(0);
  const editor = useEditor({
    extensions: [
      StarterKit.configure({
        link: false,
        underline: false,
      }),
      Underline,
      Link.configure({
        openOnClick: false,
        HTMLAttributes: {
          rel: 'noopener noreferrer',
          target: '_blank',
        },
      }),
      Placeholder.configure({ placeholder }),
    ],
    content: normalizedHtml(value),
    autofocus: autoFocus ? 'start' : false,
    editorProps: {
      attributes: {
        id,
        role: 'textbox',
        'aria-label': ariaLabel,
        class: 'min-h-[22rem] flex-1 px-4 py-4 text-base leading-relaxed outline-none prose-mirror-body',
      },
    },
    onUpdate: ({ editor: currentEditor }) => onChange(currentEditor.getHTML()),
    onCreate: ({ editor: currentEditor }) => {
      if (typeof window === 'undefined') return;
      const updates = window.__HAIL_TEST_EDITOR_UPDATES__ ?? new WeakMap<HTMLElement, (html: string) => void>();
      updates.set(currentEditor.view.dom, (html: string) => {
        currentEditor.commands.setContent(normalizedHtml(html));
      });
      window.__HAIL_TEST_EDITOR_UPDATES__ = updates;
    },
    onSelectionUpdate: () => forceToolbarUpdate((current) => current + 1),
    onTransaction: () => forceToolbarUpdate((current) => current + 1),
  });

  useImperativeHandle(ref, () => ({
    focus: (position = 'end') => {
      window.requestAnimationFrame(() => {
        editor?.chain().focus(position).run();
      });
    },
  }), [editor]);

  useEffect(() => {
    if (!editor) return;
    onReady?.(editor);
  }, [editor, onReady]);

  useEffect(() => {
    if (!editor) return;
    const nextHtml = normalizedHtml(value);
    if (editor.getHTML() === nextHtml) return;
    editor.commands.setContent(nextHtml, { emitUpdate: false });
  }, [editor, value]);

  return (
    <div className="flex min-h-[22rem] flex-1 flex-col rounded-lg border border-border bg-background">
      <ComposerToolbar editor={editor} />
      <EditorContent editor={editor} className="flex min-h-0 flex-1 flex-col [&_.ProseMirror:empty:before]:content-[attr(data-placeholder)] [&_.ProseMirror:empty:before]:pointer-events-none [&_.ProseMirror:empty:before]:float-left [&_.ProseMirror:empty:before]:h-0 [&_.ProseMirror:empty:before]:text-muted-foreground [&_.ProseMirror_blockquote]:border-l-2 [&_.ProseMirror_blockquote]:border-border [&_.ProseMirror_blockquote]:pl-4 [&_.ProseMirror_code]:rounded [&_.ProseMirror_code]:bg-muted [&_.ProseMirror_code]:px-1 [&_.ProseMirror_code]:py-0.5 [&_.ProseMirror_h2]:text-xl [&_.ProseMirror_h2]:font-semibold [&_.ProseMirror_h3]:text-lg [&_.ProseMirror_h3]:font-semibold [&_.ProseMirror_hr]:my-4 [&_.ProseMirror_hr]:border-border [&_.ProseMirror_ol]:list-decimal [&_.ProseMirror_ol]:pl-6 [&_.ProseMirror_p]:my-2 [&_.ProseMirror_pre]:overflow-x-auto [&_.ProseMirror_pre]:rounded-lg [&_.ProseMirror_pre]:bg-muted [&_.ProseMirror_pre]:p-3 [&_.ProseMirror_ul]:list-disc [&_.ProseMirror_ul]:pl-6" />
    </div>
  );
});

function ComposerToolbar({ editor }: { editor: Editor | null }) {
  const [linkOpen, setLinkOpen] = useState(false);
  const [linkHref, setLinkHref] = useState('');

  const run = (command: (editor: Editor) => boolean) => {
    if (!editor) return;
    command(editor);
  };

  const canRun = (command: (editor: Editor) => boolean) => Boolean(editor && command(editor));

  const applyLink = () => {
    if (!editor) return;
    const href = linkHref.trim();
    if (href.length === 0) {
      editor.chain().focus().extendMarkRange('link').unsetLink().run();
    } else {
      editor.chain().focus().extendMarkRange('link').setLink({ href }).run();
    }
    setLinkOpen(false);
  };

  return (
    <div className="flex flex-wrap items-center gap-1 border-b border-border px-2 py-1">
      <ToggleGroup type="multiple" variant="default" size="sm" spacing={1} aria-label="Text formatting">
        <ToggleGroupItem
          value="bold"
          aria-label="Bold"
          data-state={editor?.isActive('bold') ? 'on' : 'off'}
          disabled={!canRun((current) => current.can().chain().focus().toggleBold().run())}
          onClick={() => run((current) => current.chain().focus().toggleBold().run())}
        >
          <Bold data-icon="inline-start" />
        </ToggleGroupItem>
        <ToggleGroupItem
          value="italic"
          aria-label="Italic"
          data-state={editor?.isActive('italic') ? 'on' : 'off'}
          disabled={!canRun((current) => current.can().chain().focus().toggleItalic().run())}
          onClick={() => run((current) => current.chain().focus().toggleItalic().run())}
        >
          <Italic data-icon="inline-start" />
        </ToggleGroupItem>
        <ToggleGroupItem
          value="underline"
          aria-label="Underline"
          data-state={editor?.isActive('underline') ? 'on' : 'off'}
          disabled={!canRun((current) => current.can().chain().focus().toggleUnderline().run())}
          onClick={() => run((current) => current.chain().focus().toggleUnderline().run())}
        >
          <UnderlineIcon data-icon="inline-start" />
        </ToggleGroupItem>
        <ToggleGroupItem
          value="strike"
          aria-label="Strike"
          data-state={editor?.isActive('strike') ? 'on' : 'off'}
          disabled={!canRun((current) => current.can().chain().focus().toggleStrike().run())}
          onClick={() => run((current) => current.chain().focus().toggleStrike().run())}
        >
          <Strikethrough data-icon="inline-start" />
        </ToggleGroupItem>
      </ToggleGroup>

      <Separator orientation="vertical" className="mx-1 h-5" />

      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <Button type="button" variant="ghost" size="sm" disabled={!editor} aria-label="Heading">
            Heading
          </Button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="start">
          <DropdownMenuGroup>
            <DropdownMenuItem onClick={() => run((current) => current.chain().focus().setParagraph().run())}>Paragraph</DropdownMenuItem>
            <DropdownMenuItem onClick={() => run((current) => current.chain().focus().toggleHeading({ level: 2 }).run())}>
              <Heading2 data-icon="inline-start" />
              Heading 2
            </DropdownMenuItem>
            <DropdownMenuItem onClick={() => run((current) => current.chain().focus().toggleHeading({ level: 3 }).run())}>
              <Heading3 data-icon="inline-start" />
              Heading 3
            </DropdownMenuItem>
          </DropdownMenuGroup>
        </DropdownMenuContent>
      </DropdownMenu>

      <ToggleGroup type="multiple" variant="default" size="sm" spacing={1} aria-label="Blocks">
        <ToggleGroupItem
          value="bullet-list"
          aria-label="Bullet list"
          data-state={editor?.isActive('bulletList') ? 'on' : 'off'}
          disabled={!canRun((current) => current.can().chain().focus().toggleBulletList().run())}
          onClick={() => run((current) => current.chain().focus().toggleBulletList().run())}
        >
          <List data-icon="inline-start" />
        </ToggleGroupItem>
        <ToggleGroupItem
          value="ordered-list"
          aria-label="Ordered list"
          data-state={editor?.isActive('orderedList') ? 'on' : 'off'}
          disabled={!canRun((current) => current.can().chain().focus().toggleOrderedList().run())}
          onClick={() => run((current) => current.chain().focus().toggleOrderedList().run())}
        >
          <ListOrdered data-icon="inline-start" />
        </ToggleGroupItem>
        <ToggleGroupItem
          value="blockquote"
          aria-label="Blockquote"
          data-state={editor?.isActive('blockquote') ? 'on' : 'off'}
          disabled={!canRun((current) => current.can().chain().focus().toggleBlockquote().run())}
          onClick={() => run((current) => current.chain().focus().toggleBlockquote().run())}
        >
          <Quote data-icon="inline-start" />
        </ToggleGroupItem>
        <ToggleGroupItem
          value="code"
          aria-label="Inline code"
          data-state={editor?.isActive('code') ? 'on' : 'off'}
          disabled={!canRun((current) => current.can().chain().focus().toggleCode().run())}
          onClick={() => run((current) => current.chain().focus().toggleCode().run())}
        >
          <Code data-icon="inline-start" />
        </ToggleGroupItem>
      </ToggleGroup>

      <Button
        type="button"
        variant="ghost"
        size="sm"
        disabled={!canRun((current) => current.can().chain().focus().toggleCodeBlock().run())}
        onClick={() => run((current) => current.chain().focus().toggleCodeBlock().run())}
      >
        Code block
      </Button>
      <Button
        type="button"
        variant="ghost"
        size="icon-sm"
        aria-label="Horizontal rule"
        disabled={!canRun((current) => current.can().chain().focus().setHorizontalRule().run())}
        onClick={() => run((current) => current.chain().focus().setHorizontalRule().run())}
      >
        <Minus data-icon="inline-start" />
      </Button>

      <Separator orientation="vertical" className="mx-1 h-5" />

      <Popover open={linkOpen} onOpenChange={(open) => {
        setLinkOpen(open);
        if (open && editor) setLinkHref(editor.getAttributes('link').href ?? '');
      }}>
        <PopoverTrigger asChild>
          <Button type="button" variant="ghost" size="icon-sm" disabled={!editor} aria-label="Link">
            <LinkIcon data-icon="inline-start" />
          </Button>
        </PopoverTrigger>
        <PopoverContent align="start" className="w-80">
          <div className="flex items-center gap-2">
            <Input
              value={linkHref}
              onChange={(event) => setLinkHref(event.target.value)}
              placeholder="https://example.com"
              aria-label="Link URL"
              onKeyDown={(event) => {
                if (event.key === 'Enter') applyLink();
              }}
            />
            <Button type="button" size="sm" onClick={applyLink}>Apply</Button>
            <Button type="button" size="sm" variant="ghost" onClick={() => setLinkHref('')} aria-label="Clear link">
              <RemoveFormatting data-icon="inline-start" />
            </Button>
          </div>
        </PopoverContent>
      </Popover>

      <Separator orientation="vertical" className="mx-1 h-5" />

      <Button
        type="button"
        variant="ghost"
        size="icon-sm"
        aria-label="Undo"
        disabled={!canRun((current) => current.can().chain().focus().undo().run())}
        onClick={() => run((current) => current.chain().focus().undo().run())}
      >
        <Undo2 data-icon="inline-start" />
      </Button>
      <Button
        type="button"
        variant="ghost"
        size="icon-sm"
        aria-label="Redo"
        disabled={!canRun((current) => current.can().chain().focus().redo().run())}
        onClick={() => run((current) => current.chain().focus().redo().run())}
      >
        <Redo2 data-icon="inline-start" />
      </Button>
    </div>
  );
}
