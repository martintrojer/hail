import { forwardRef, useEffect, useImperativeHandle, useRef, useState } from 'react';
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
import { Toggle } from '../ui/toggle';

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

const allowedPasteTags = new Set([
  'a',
  'blockquote',
  'br',
  'code',
  'em',
  'h1',
  'h2',
  'h3',
  'hr',
  'i',
  'li',
  'ol',
  'p',
  'pre',
  's',
  'strike',
  'strong',
  'u',
  'ul',
]);

const droppedPasteTags = new Set([
  'button',
  'embed',
  'form',
  'iframe',
  'input',
  'link',
  'meta',
  'object',
  'script',
  'select',
  'style',
  'textarea',
]);

const urlMatcher = /\b((?:https?:\/\/|www\.)[^\s<]+)/gi;

function escapeHtml(value: string) {
  return value.replace(/[&<>"']/g, (character) => {
    switch (character) {
      case '&':
        return '&amp;';
      case '<':
        return '&lt;';
      case '>':
        return '&gt;';
      case '"':
        return '&quot;';
      case "'":
        return '&#39;';
      default:
        return character;
    }
  });
}

function normalizeSafeUrl(value: string) {
  const trimmed = value.trim();
  if (!trimmed) return null;
  const candidate = /^www\./i.test(trimmed) ? `https://${trimmed}` : trimmed;

  try {
    const parsed = new URL(candidate);
    if (parsed.protocol === 'http:' || parsed.protocol === 'https:' || parsed.protocol === 'mailto:') {
      return parsed.toString();
    }
  } catch {
    return null;
  }

  return null;
}

function splitTrailingUrlPunctuation(value: string) {
  const match = value.match(/[),.!?:;]+$/);
  if (!match) return [value, ''] as const;
  const trailing = match[0];
  return [value.slice(0, -trailing.length), trailing] as const;
}

function plainTextToHtmlWithAutoLinks(text: string) {
  const lines = text.split(/\r?\n/);
  return lines.map((line) => {
    let linkedLine = '';
    let lastIndex = 0;
    urlMatcher.lastIndex = 0;

    for (const match of line.matchAll(urlMatcher)) {
      const rawUrl = match[0];
      const matchIndex = match.index ?? 0;
      linkedLine += escapeHtml(line.slice(lastIndex, matchIndex));

      const [urlWithoutPunctuation, trailing] = splitTrailingUrlPunctuation(rawUrl);
      const href = normalizeSafeUrl(urlWithoutPunctuation);
      if (href) {
        linkedLine += `<a href="${escapeHtml(href)}" rel="noopener noreferrer" target="_blank">${escapeHtml(urlWithoutPunctuation)}</a>${escapeHtml(trailing)}`;
      } else {
        linkedLine += escapeHtml(rawUrl);
      }
      lastIndex = matchIndex + rawUrl.length;
    }

    linkedLine += escapeHtml(line.slice(lastIndex));
    return `<p>${linkedLine || '<br>'}</p>`;
  }).join('');
}

function plainTextHasAutoLinkableUrl(text: string) {
  urlMatcher.lastIndex = 0;
  return urlMatcher.test(text);
}

function sanitizePastedHtml(html: string) {
  if (typeof window === 'undefined') return '';

  const documentFromPaste = new DOMParser().parseFromString(html, 'text/html');
  const output = document.createElement('div');

  function appendCleanChildren(source: Node, target: Node) {
    for (const child of Array.from(source.childNodes)) {
      const cleanChild = cleanNode(child);
      if (cleanChild) target.appendChild(cleanChild);
    }
  }

  function cleanNode(node: Node): Node | null {
    if (node.nodeType === Node.TEXT_NODE) {
      return document.createTextNode(node.textContent ?? '');
    }

    if (node.nodeType !== Node.ELEMENT_NODE) {
      return null;
    }

    const element = node as Element;
    const tagName = element.tagName.toLowerCase();

    if (droppedPasteTags.has(tagName)) {
      return null;
    }

    if (!allowedPasteTags.has(tagName)) {
      const fragment = document.createDocumentFragment();
      appendCleanChildren(element, fragment);
      return fragment;
    }

    const cleanElement = document.createElement(tagName);
    if (tagName === 'a') {
      const href = normalizeSafeUrl(element.getAttribute('href') ?? '');
      if (!href) {
        const fragment = document.createDocumentFragment();
        appendCleanChildren(element, fragment);
        return fragment;
      }
      cleanElement.setAttribute('href', href);
      cleanElement.setAttribute('rel', 'noopener noreferrer');
      cleanElement.setAttribute('target', '_blank');
    }

    appendCleanChildren(element, cleanElement);
    return cleanElement;
  }

  appendCleanChildren(documentFromPaste.body, output);
  return output.innerHTML;
}

function applyKeyboardShortcut(editor: Editor, event: KeyboardEvent) {
  const modifierPressed = event.metaKey || event.ctrlKey;
  if (!modifierPressed || event.altKey) return false;

  const key = event.key.toLowerCase();
  const code = event.code;
  const chain = () => editor.chain().focus();

  if (!event.shiftKey && key === 'b') return chain().toggleBold().run();
  if (!event.shiftKey && key === 'i') return chain().toggleItalic().run();
  if (!event.shiftKey && key === 'u') return chain().toggleUnderline().run();
  if (!event.shiftKey && key === 'e') return chain().toggleCode().run();
  if (event.shiftKey && (key === '7' || key === '&' || code === 'Digit7')) return chain().toggleOrderedList().run();
  if (event.shiftKey && (key === '8' || key === '*' || code === 'Digit8')) return chain().toggleBulletList().run();
  if (event.shiftKey && (key === '.' || key === '>' || code === 'Period')) return chain().toggleBlockquote().run();
  if (event.shiftKey && key === 'c') return chain().toggleCodeBlock().run();
  if (!event.shiftKey && key === 'z') return chain().undo().run();
  if (event.shiftKey && key === 'z') return chain().redo().run();

  return false;
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
  const openLinkPopoverRef = useRef<() => void>(() => undefined);
  const editorRef = useRef<Editor | null>(null);
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
      handlePaste(_view, event) {
        const html = event.clipboardData?.getData('text/html');
        const text = event.clipboardData?.getData('text/plain') ?? '';

        if (html) {
          const sanitizedHtml = sanitizePastedHtml(html);
          if (!sanitizedHtml) return false;
          editorRef.current?.chain().focus().insertContent(sanitizedHtml).run();
          return true;
        }

        if (text && plainTextHasAutoLinkableUrl(text)) {
          editorRef.current?.chain().focus().insertContent(plainTextToHtmlWithAutoLinks(text)).run();
          return true;
        }

        return false;
      },
      handleKeyDown(_view, event) {
        if ((event.metaKey || event.ctrlKey) && !event.shiftKey && !event.altKey && event.key.toLowerCase() === 'k') {
          event.preventDefault();
          openLinkPopoverRef.current();
          return true;
        }

        if (editorRef.current && applyKeyboardShortcut(editorRef.current, event)) {
          event.preventDefault();
          return true;
        }

        return false;
      },
    },
    onUpdate: ({ editor: currentEditor }) => onChange(currentEditor.getHTML()),
    onCreate: ({ editor: currentEditor }) => {
      if (typeof window === 'undefined') return;
      const updates = window.__HAIL_TEST_EDITOR_UPDATES__ ?? new WeakMap<HTMLElement, (html: string) => void>();
      updates.set(currentEditor.view.dom, (html: string) => {
        if (currentEditor.isDestroyed) return;
        currentEditor.commands.setContent(normalizedHtml(html));
      });
      window.__HAIL_TEST_EDITOR_UPDATES__ = updates;
      const editors = window.__HAIL_TEST_EDITORS__ ?? new WeakMap<HTMLElement, Editor>();
      editors.set(currentEditor.view.dom, currentEditor);
      window.__HAIL_TEST_EDITORS__ = editors;
    },
    onSelectionUpdate: () => forceToolbarUpdate((current) => current + 1),
    onTransaction: () => forceToolbarUpdate((current) => current + 1),
  });

  editorRef.current = editor;

  useImperativeHandle(ref, () => ({
    focus: (position = 'end') => {
      window.requestAnimationFrame(() => {
        if (!editor || editor.isDestroyed) return;
        editor.chain().focus(position).run();
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
      <ComposerToolbar editor={editor} onOpenLinkShortcut={(openLink) => {
        openLinkPopoverRef.current = openLink;
      }} />
      <EditorContent editor={editor} className="flex min-h-0 flex-1 flex-col [&_.ProseMirror:empty:before]:content-[attr(data-placeholder)] [&_.ProseMirror:empty:before]:pointer-events-none [&_.ProseMirror:empty:before]:float-left [&_.ProseMirror:empty:before]:h-0 [&_.ProseMirror:empty:before]:text-muted-foreground [&_.ProseMirror_blockquote]:border-l-2 [&_.ProseMirror_blockquote]:border-border [&_.ProseMirror_blockquote]:pl-4 [&_.ProseMirror_code]:rounded [&_.ProseMirror_code]:bg-muted [&_.ProseMirror_code]:px-1 [&_.ProseMirror_code]:py-0.5 [&_.ProseMirror_h2]:text-xl [&_.ProseMirror_h2]:font-semibold [&_.ProseMirror_h3]:text-lg [&_.ProseMirror_h3]:font-semibold [&_.ProseMirror_hr]:my-4 [&_.ProseMirror_hr]:border-border [&_.ProseMirror_ol]:list-decimal [&_.ProseMirror_ol]:pl-6 [&_.ProseMirror_p]:my-2 [&_.ProseMirror_pre]:overflow-x-auto [&_.ProseMirror_pre]:rounded-lg [&_.ProseMirror_pre]:bg-muted [&_.ProseMirror_pre]:p-3 [&_.ProseMirror_ul]:list-disc [&_.ProseMirror_ul]:pl-6" />
    </div>
  );
});

function ComposerToolbar({ editor, onOpenLinkShortcut }: { editor: Editor | null; onOpenLinkShortcut: (open: () => void) => void }) {
  const [linkOpen, setLinkOpen] = useState(false);
  const [linkHref, setLinkHref] = useState('');
  const [linkError, setLinkError] = useState<string | null>(null);
  const linkSelectionRef = useRef<{ from: number; to: number } | null>(null);

  const run = (command: (editor: Editor) => boolean) => {
    if (!editor) return;
    command(editor);
  };

  const canRun = (command: (editor: Editor) => boolean) => Boolean(editor && command(editor));

  const openLinkEditor = () => {
    if (!editor) return;
    linkSelectionRef.current = {
      from: editor.state.selection.from,
      to: editor.state.selection.to,
    };
    setLinkHref(editor.getAttributes('link').href ?? '');
    setLinkError(null);
    setLinkOpen(true);
  };

  useEffect(() => {
    onOpenLinkShortcut(openLinkEditor);
  });

  const applyLink = () => {
    if (!editor) return;
    const href = linkHref.trim();
    const selection = linkSelectionRef.current;
    const linkChain = editor.chain().focus();
    if (selection) linkChain.setTextSelection(selection);

    if (href.length === 0) {
      linkChain.extendMarkRange('link').unsetLink().run();
      setLinkError(null);
      setLinkOpen(false);
      return;
    }

    const safeHref = normalizeSafeUrl(href);
    if (!safeHref) {
      setLinkError('Enter an http, https, or mailto URL.');
      return;
    }

    linkChain.extendMarkRange('link').setLink({
      href: safeHref,
      rel: 'noopener noreferrer',
      target: '_blank',
    }).run();
    setLinkHref(safeHref);
    setLinkError(null);
    setLinkOpen(false);
  };

  const removeLink = () => {
    if (!editor) return;
    const selection = linkSelectionRef.current;
    const linkChain = editor.chain().focus();
    if (selection) linkChain.setTextSelection(selection);
    linkChain.extendMarkRange('link').unsetLink().run();
    setLinkHref('');
    setLinkError(null);
    setLinkOpen(false);
  };

  return (
    <div className="flex flex-wrap items-center gap-1 border-b border-border px-2 py-1">
      <div className="flex w-fit items-center gap-1" role="group" aria-label="Text formatting">
        <Toggle
          type="button"
          variant="default"
          size="sm"
          aria-label="Bold"
          pressed={Boolean(editor?.isActive('bold'))}
          disabled={!canRun((current) => current.can().chain().focus().toggleBold().run())}
          onPressedChange={() => run((current) => current.chain().focus().toggleBold().run())}
        >
          <Bold data-icon="inline-start" />
        </Toggle>
        <Toggle
          type="button"
          variant="default"
          size="sm"
          aria-label="Italic"
          pressed={Boolean(editor?.isActive('italic'))}
          disabled={!canRun((current) => current.can().chain().focus().toggleItalic().run())}
          onPressedChange={() => run((current) => current.chain().focus().toggleItalic().run())}
        >
          <Italic data-icon="inline-start" />
        </Toggle>
        <Toggle
          type="button"
          variant="default"
          size="sm"
          aria-label="Underline"
          pressed={Boolean(editor?.isActive('underline'))}
          disabled={!canRun((current) => current.can().chain().focus().toggleUnderline().run())}
          onPressedChange={() => run((current) => current.chain().focus().toggleUnderline().run())}
        >
          <UnderlineIcon data-icon="inline-start" />
        </Toggle>
        <Toggle
          type="button"
          variant="default"
          size="sm"
          aria-label="Strike"
          pressed={Boolean(editor?.isActive('strike'))}
          disabled={!canRun((current) => current.can().chain().focus().toggleStrike().run())}
          onPressedChange={() => run((current) => current.chain().focus().toggleStrike().run())}
        >
          <Strikethrough data-icon="inline-start" />
        </Toggle>
      </div>

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

      <div className="flex w-fit items-center gap-1" role="group" aria-label="Blocks">
        <Toggle
          type="button"
          variant="default"
          size="sm"
          aria-label="Bullet list"
          pressed={Boolean(editor?.isActive('bulletList'))}
          disabled={!canRun((current) => current.can().chain().focus().toggleBulletList().run())}
          onPressedChange={() => run((current) => current.chain().focus().toggleBulletList().run())}
        >
          <List data-icon="inline-start" />
        </Toggle>
        <Toggle
          type="button"
          variant="default"
          size="sm"
          aria-label="Ordered list"
          pressed={Boolean(editor?.isActive('orderedList'))}
          disabled={!canRun((current) => current.can().chain().focus().toggleOrderedList().run())}
          onPressedChange={() => run((current) => current.chain().focus().toggleOrderedList().run())}
        >
          <ListOrdered data-icon="inline-start" />
        </Toggle>
        <Toggle
          type="button"
          variant="default"
          size="sm"
          aria-label="Blockquote"
          pressed={Boolean(editor?.isActive('blockquote'))}
          disabled={!canRun((current) => current.can().chain().focus().toggleBlockquote().run())}
          onPressedChange={() => run((current) => current.chain().focus().toggleBlockquote().run())}
        >
          <Quote data-icon="inline-start" />
        </Toggle>
        <Toggle
          type="button"
          variant="default"
          size="sm"
          aria-label="Inline code"
          pressed={Boolean(editor?.isActive('code'))}
          disabled={!canRun((current) => current.can().chain().focus().toggleCode().run())}
          onPressedChange={() => run((current) => current.chain().focus().toggleCode().run())}
        >
          <Code data-icon="inline-start" />
        </Toggle>
      </div>

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
        if (open) {
          openLinkEditor();
        } else {
          setLinkOpen(false);
          setLinkError(null);
        }
      }}>
        <PopoverTrigger asChild>
          <Button type="button" variant="ghost" size="icon-sm" disabled={!editor} aria-label="Link" onClick={openLinkEditor}>
            <LinkIcon data-icon="inline-start" />
          </Button>
        </PopoverTrigger>
        <PopoverContent align="start" className="w-96">
          <div className="flex flex-col gap-2">
            <div className="flex items-center gap-2">
              <Input
                value={linkHref}
                onChange={(event) => {
                  setLinkHref(event.target.value);
                  setLinkError(null);
                }}
                placeholder="https://example.com"
                aria-label="Link URL"
                aria-invalid={Boolean(linkError)}
                onKeyDown={(event) => {
                  if (event.key === 'Enter') applyLink();
                }}
              />
              <Button type="button" size="sm" onClick={applyLink}>Apply</Button>
              <Button type="button" size="sm" variant="ghost" onClick={removeLink}>Remove</Button>
            </div>
            {linkError ? <p className="text-xs text-destructive">{linkError}</p> : null}
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
