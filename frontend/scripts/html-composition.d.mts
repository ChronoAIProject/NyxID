export interface HtmlInline { kind: string; sha256: string; size: number }
export interface HtmlReference { kind: string; url: string }
export function contentHash(text: string): string;
export function htmlContents(html: string): { inline: HtmlInline[]; references: HtmlReference[] };
export function singlefileContent(kind: string, code: string): string;
