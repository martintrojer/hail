import { useState } from 'react';
import type { Editor } from '@tiptap/react';
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeAll, describe, expect, it } from 'vitest';
import { RichTextEditor } from './RichTextEditor';

beforeAll(() => {
  Element.prototype.getClientRects = function getClientRects() {
    const rect = this.getBoundingClientRect();
    return {
      length: 1,
      item: (index: number) => (index === 0 ? rect : null),
      0: rect,
      [Symbol.iterator]: function* iterator() {
        yield rect;
      },
    } as DOMRectList;
  };
  document.elementFromPoint = () => document.body;
  Range.prototype.getClientRects = function getRangeClientRects() {
    const rect = document.body.getBoundingClientRect();
    return {
      length: 1,
      item: (index: number) => (index === 0 ? rect : null),
      0: rect,
      [Symbol.iterator]: function* iterator() {
        yield rect;
      },
    } as DOMRectList;
  };
  Range.prototype.getBoundingClientRect = () => document.body.getBoundingClientRect();
});

function renderControlledEditor() {
  let currentEditor: Editor | null = null;

  function ControlledEditor() {
    const [html, setHtml] = useState('<p></p>');
    return (
      <>
        <RichTextEditor
          value={html}
          onReady={(editor) => {
            currentEditor = editor;
          }}
          onChange={setHtml}
        />
        <output aria-label="Controlled HTML">{html}</output>
      </>
    );
  }

  render(<ControlledEditor />);

  return {
    editor: () => {
      if (!currentEditor) throw new Error('editor not ready');
      return currentEditor;
    },
    controlledHtml: () => screen.getByLabelText('Controlled HTML').textContent ?? '',
  };
}

describe('RichTextEditor', () => {
  it('updates controlled HTML when bold, italic, and list toolbar buttons are toggled', async () => {
    const { editor, controlledHtml } = renderControlledEditor();
    await screen.findByLabelText('Body');
    await waitFor(() => expect(editor()).toBeTruthy());

    const boldButton = screen.getByRole('button', { name: 'Bold' });
    const italicButton = screen.getByRole('button', { name: 'Italic' });
    const bulletListButton = screen.getByRole('button', { name: 'Bullet list' });

    expect(boldButton).toHaveAttribute('aria-pressed', 'false');
    fireEvent.click(boldButton);
    await waitFor(() => expect(boldButton).toHaveAttribute('aria-pressed', 'true'));
    editor().chain().focus().insertContent('Bold words').run();
    await waitFor(() => expect(controlledHtml()).toContain('<strong>Bold words</strong>'));

    editor().chain().focus().clearContent().unsetBold().toggleItalic().run();
    await waitFor(() => expect(italicButton).toHaveAttribute('aria-pressed', 'true'));
    editor().chain().focus().insertContent('Italic words').run();
    await waitFor(() => expect(controlledHtml()).toContain('<em>Italic words</em>'));

    editor().chain().focus().clearContent().unsetItalic().toggleBulletList().run();
    await waitFor(() => expect(bulletListButton).toHaveAttribute('aria-pressed', 'true'));
    editor().chain().focus().insertContent('List item').run();
    await waitFor(() => expect(controlledHtml()).toContain('<ul>'));
    expect(controlledHtml()).toContain('<li><p>List item</p></li>');
  });

  it('drives toolbar pressed state from the editor for selection changes and keyboard shortcuts', async () => {
    const { editor, controlledHtml } = renderControlledEditor();
    const body = await screen.findByLabelText('Body');
    await waitFor(() => expect(editor()).toBeTruthy());

    const boldButton = screen.getByRole('button', { name: 'Bold' });
    expect(boldButton).toHaveAttribute('aria-pressed', 'false');

    editor().commands.setContent('<p><strong>Bold words</strong> and plain words</p>');
    editor().chain().focus().setTextSelection({ from: 2, to: 6 }).run();
    await waitFor(() => expect(boldButton).toHaveAttribute('aria-pressed', 'true'));

    editor().chain().focus().setTextSelection({ from: 19, to: 24 }).run();
    await waitFor(() => expect(boldButton).toHaveAttribute('aria-pressed', 'false'));

    fireEvent.keyDown(body, { key: 'b', code: 'KeyB', ctrlKey: true });
    await waitFor(() => expect(boldButton).toHaveAttribute('aria-pressed', 'true'));
    editor().chain().focus().insertContent('Key bold').run();
    await waitFor(() => expect(controlledHtml()).toContain('<strong>Key bold</strong>'));
  });
});
