import { useState } from 'react';
import type { Editor } from '@tiptap/react';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeAll, describe, expect, it } from 'vitest';
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

afterEach(() => {
  cleanup();
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

    fireEvent.click(screen.getByRole('button', { name: 'Bold' }));
    editor().chain().focus().insertContent('Bold words').run();
    await waitFor(() => expect(controlledHtml()).toContain('<strong>Bold words</strong>'));

    editor().commands.setContent('<p></p>');
    fireEvent.click(screen.getByRole('button', { name: 'Bold' }));
    fireEvent.click(screen.getByRole('button', { name: 'Italic' }));
    editor().chain().focus().insertContent('Italic words').run();
    await waitFor(() => expect(controlledHtml()).toContain('<em>Italic words</em>'));

    editor().commands.setContent('<p></p>');
    fireEvent.click(screen.getByRole('button', { name: 'Italic' }));
    fireEvent.click(screen.getByRole('button', { name: 'Bullet list' }));
    editor().chain().focus().insertContent('List item').run();
    await waitFor(() => expect(controlledHtml()).toContain('<ul>'));
    expect(controlledHtml()).toContain('<li><p>List item</p></li>');
  });
});
