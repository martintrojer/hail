const htmlEscapes: Record<string, string> = {
  '&': '&amp;',
  '<': '&lt;',
  '>': '&gt;',
  '"': '&quot;',
  "'": '&#39;',
};

function escapeHtml(text: string) {
  return text.replace(/[&<>"']/g, (character) => htmlEscapes[character]);
}

export function plaintextToBodyHtml(text: string) {
  const normalized = text.trim().replace(/\r\n?/g, '\n');
  if (!normalized) return '<p></p>';

  return normalized
    .split(/\n{2,}/)
    .map((paragraph) => `<p>${paragraph.split('\n').map(escapeHtml).join('<br/>')}</p>`)
    .join('');
}
