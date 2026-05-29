import { describe, expect, it } from 'vitest';
import { plaintextToBodyHtml } from './plaintextToBodyHtml';

describe('plaintextToBodyHtml', () => {
  it('returns a single empty paragraph for empty and whitespace-only input', () => {
    expect(plaintextToBodyHtml('')).toBe('<p></p>');
    expect(plaintextToBodyHtml('   \n\t\r\n  ')).toBe('<p></p>');
  });

  it('escapes HTML-sensitive characters', () => {
    expect(plaintextToBodyHtml('& < > " \'')).toBe(
      '<p>&amp; &lt; &gt; &quot; &#39;</p>',
    );
  });

  it('wraps a single line in a paragraph', () => {
    expect(plaintextToBodyHtml('Hello from hail.')).toBe('<p>Hello from hail.</p>');
  });

  it('splits paragraphs on blank lines', () => {
    expect(plaintextToBodyHtml('First paragraph.\n\nSecond paragraph.\n\nThird paragraph.')).toBe(
      '<p>First paragraph.</p><p>Second paragraph.</p><p>Third paragraph.</p>',
    );
  });

  it('converts single line breaks inside a paragraph to br elements', () => {
    expect(plaintextToBodyHtml('Line one\nLine two\nLine three')).toBe(
      '<p>Line one<br/>Line two<br/>Line three</p>',
    );
  });

  it('normalizes CRLF and mixed line endings before paragraph conversion', () => {
    expect(plaintextToBodyHtml('First line\r\nSecond line\rThird line\n\nNext paragraph')).toBe(
      '<p>First line<br/>Second line<br/>Third line</p><p>Next paragraph</p>',
    );
  });

  it('escapes a real-world reply blurb while preserving line breaks', () => {
    expect(plaintextToBodyHtml('if x < y > 0 && z == 1\nthen...')).toBe(
      '<p>if x &lt; y &gt; 0 &amp;&amp; z == 1<br/>then...</p>',
    );
  });
});
