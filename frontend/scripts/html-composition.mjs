import { parse } from 'parse5';
import { createHash } from 'node:crypto';
export const contentHash = text => createHash('sha256').update(text).digest('hex');
/** Browser-compatible tokenization, including character references and raw text. */
export function htmlContents(html) {
  const document = parse(html, { sourceCodeLocationInfo: true });
  const inline = [], references = [];
  const visit = node => {
    if (node.tagName === 'base') throw new Error('Unmapped HTML base element');
    for (const attribute of node.attrs ?? []) {
      if (/^on/i.test(attribute.name) || attribute.name === 'srcdoc' || /^\s*javascript:/i.test(attribute.value)) {
        throw new Error('Unmapped active HTML attribute: ' + attribute.name);
      }
      if (attribute.name === 'style') inline.push({ kind: 'style-attribute', sha256: contentHash(attribute.value), size: Buffer.byteLength(attribute.value) });
    }
    const attrs = Object.fromEntries((node.attrs ?? []).map(attribute => [attribute.name, attribute.value]));
    if (node.tagName === 'script' || node.tagName === 'style') {
      const kind = node.tagName;
      if (kind === 'script' && attrs.src) references.push({ kind, url: attrs.src });
      else {
        const location = node.sourceCodeLocation;
        if (!location?.startTag || !location.endTag) throw new Error('Unclosed executable HTML block');
        const content = html.slice(location.startTag.endOffset, location.endTag.startOffset);
        if (content.trim()) inline.push({ kind, sha256: contentHash(content), size: Buffer.byteLength(content) });
      }
    }
    const rel = (attrs.rel ?? '').toLowerCase().split(/\s+/);
    if (node.tagName === 'link' && (rel.includes('stylesheet') || rel.includes('modulepreload'))) {
      references.push({ kind: rel.includes('stylesheet') ? 'style' : 'script', url: attrs.href });
    }
    for (const child of node.childNodes ?? []) visit(child);
    // Template content is inert until code instantiates it; recording/rejecting
    // active content there avoids an unaccounted script when it is instantiated.
    if (node.content) visit(node.content);
  };
  visit(document);
  return { inline, references };
}
export function singlefileContent(kind, code) {
  return kind === 'script'
    ? code.replace(/"?__VITE_PRELOAD__"?/g, 'void 0').replace(/<(\/script>|!--)/g, '\\x3C$1').trim()
    : code.replace('@charset "UTF-8";', '').trim();
}
