import type MarkdownIt from "markdown-it";
import imageRule from "markdown-it/lib/rules_inline/image";
import footnote from "markdown-it-footnote";
import { NYXID_FRONTEND_URL } from "../../lib/env";
import { assistantLinkTarget } from "./assistantLinkTarget";

export const assistantMarkdownOptions = {
  typographer: true,
  linkify: true,
  html: false,
};

export function safeAssistantLinkUrl(
  url: string,
  frontendUrl = NYXID_FRONTEND_URL,
): string | null {
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
              .trim() || "Image";
          child.children = null;
        }
        footnoteLink(child);
      }
    }
    return true;
  });
  return parser;
}
