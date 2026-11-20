import { bareAutolinkTarget, splitBareAutolinks } from "./autolinkPunctuation";
import { translateApp } from "../../i18n/appTranslation";
import type MarkdownIt from "markdown-it";
import imageRule from "markdown-it/lib/rules_inline/image.mjs";
import footnote from "markdown-it-footnote";
import { firstPartyFrontendUrls } from "../../lib/env";
import { assistantLinkTarget } from "./assistantLinkTarget";

export const assistantMarkdownOptions = {
  typographer: true,
  linkify: true,
  html: false,
};

export function safeAssistantLinkUrl(
  url: string,
  frontendUrl: string | readonly string[] = firstPartyFrontendUrls(),
  bareAutolink = false,
): string | null {
  if (bareAutolink) url = bareAutolinkTarget(url);
  if (url.trim().startsWith("#")) return url.trim();
  const internal = assistantLinkTarget(url, frontendUrl);
  if (internal) return internal.href;
  try {
    const parsed = new URL(url);
    if (
      (parsed.protocol === "https:" || parsed.protocol === "mailto:") &&
      !parsed.username &&
      !parsed.password
    )
      return parsed.href;
  } catch {
    // Unsupported or malformed model-provided destinations remain plain text.
  }
  return null;
}

/** Convert GFM extensions to native tokens, never HTML or remote image loads. */
export function configureAssistantMarkdown(parser: MarkdownIt): MarkdownIt {
  parser.validateLink = (url) => safeAssistantLinkUrl(url) !== null;
  parser.use(footnote);
  // Resolve bare URLs after inline Markdown delimiters have been parsed.
  parser.inline.ruler.disable("linkify");
  parser.core.ruler.after("linkify", "cjk_autolink_punctuation", (state) => {
    for (const token of state.tokens) {
      const children = token.children;
      if (!children) continue;
      for (let index = 0; index < children.length - 2; index++) {
        const open = children[index]!;
        const text = children[index + 1]!;
        const close = children[index + 2]!;
        if (
          open.type !== "link_open" ||
          open.markup !== "linkify" ||
          text.type !== "text" ||
          close.type !== "link_close"
        )
          continue;
        open.attrSet("data-nyx-bare-autolink", "true");
        if (!/[^\p{ASCII}]/u.test(text.content)) continue;
        const replacements = splitBareAutolinks(text.content).flatMap(
          (part) => {
            const plain = (value: string) => {
              const label = new state.Token("text", "", 0);
              label.content = value;
              return label;
            };
            if (!part.href) return [plain(part.text)];
            const matches = state.md.linkify.match(part.text) ?? [];
            const result = [];
            let cursor = 0;
            for (const match of matches) {
              if (match.index > cursor)
                result.push(plain(part.text.slice(cursor, match.index)));
              const href = state.md.normalizeLink(match.url);
              const label = plain(match.raw);
              if (state.md.validateLink(href)) {
                const link = new state.Token("link_open", "a", 1);
                link.markup = "linkify";
                link.info = "auto";
                link.attrSet("href", href);
                link.attrSet("data-nyx-bare-autolink", "true");
                const end = new state.Token("link_close", "a", -1);
                end.markup = "linkify";
                end.info = "auto";
                result.push(link, label, end);
              } else result.push(label);
              cursor = match.lastIndex;
            }
            if (cursor < part.text.length)
              result.push(plain(part.text.slice(cursor)));
            return result;
          },
        );
        children.splice(index, 3, ...replacements);
        index += replacements.length - 1;
      }
    }
    return true;
  });
  parser.inline.ruler.disable("footnote_inline");
  // Parse image labels even when their URL is unsafe. The native renderer
  // literalizes them, matching the web sanitizer, without fetching any bytes.
  parser.inline.ruler.at("image", (state, silent) => {
    const validate = state.md.validateLink;
    state.md.validateLink = () => true;
    try {
      return imageRule(state, silent);
    } finally {
      state.md.validateLink = validate;
    }
  });
  parser.core.ruler.push("native_safe_gfm", (state) => {
    const items: (typeof state.tokens)[number][] = [];
    const footnoteLink = (token: (typeof state.tokens)[number]) => {
      if (token.type !== "footnote_ref" && token.type !== "footnote_anchor")
        return;
      const id = Number(token.meta?.id ?? 0) + 1;
      const suffix = token.meta?.subId ? `:${token.meta.subId}` : "";
      token.attrSet(
        "href",
        token.type === "footnote_ref" ? `#fn${id}` : `#fnref${id}${suffix}`,
      );
      if (token.type === "footnote_ref")
        token.attrSet("id", `fnref${id}${suffix}`);
      token.attrSet("data-number", String(id));
    };
    for (const [index, token] of state.tokens.entries()) {
      if (token.type === "list_item_open") items.push(token);
      if (token.type === "list_item_close") items.pop();
      const first = token.children?.[0];
      if (
        token.type === "inline" &&
        state.tokens[index - 1]?.type === "paragraph_open" &&
        state.tokens[index - 2]?.type === "list_item_open" &&
        first?.type === "text"
      ) {
        const task = /^\[([ xX])\](?:[ \t]+|$)/.exec(first.content);
        if (task && /^\[([ xX])\](?:[ \t]+|$)/.test(token.content)) {
          items
            .at(-1)
            ?.attrSet("data-task-checked", task[1] === " " ? "false" : "true");
          first.content = first.content.slice(task[0].length);
        }
      }
      if (token.type.startsWith("footnote_")) {
        const id = Number(token.meta?.id ?? 0) + 1;
        if (token.type === "footnote_open") {
          token.attrSet("id", `fn${id}`);
          token.attrSet("data-number", String(id));
        }
        token.block = token.type !== "footnote_anchor";
      }
      footnoteLink(token);
      let insideLink = false;
      for (const child of token.children ?? []) {
        if (child.type === "link_open") insideLink = true;
        if (child.type === "link_close") insideLink = false;
        if (child.type === "image" && insideLink) {
          child.type = "text";
          child.content =
            child.children
              ?.map((part) => part.content)
              .join("")
              .trim() || translateApp("Image");
          child.children = null;
        }
        footnoteLink(child);
      }
    }
    return true;
  });
  return parser;
}
