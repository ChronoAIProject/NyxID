declare module "markdown-it/lib/rules_inline/image.mjs" {
  import type { RuleInline } from "markdown-it/lib/parser_inline.mjs";
  const image: RuleInline;
  export default image;
}

declare module "markdown-it-footnote" {
  import type MarkdownIt from "markdown-it";
  export default function footnote(parser: MarkdownIt): void;
}
