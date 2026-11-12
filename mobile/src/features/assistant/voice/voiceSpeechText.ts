import MarkdownIt from "markdown-it";
import type Token from "markdown-it/lib/token";
import { sanitizeAssistantMessageContent } from "../assistantChatContent";

const markdown = new MarkdownIt({ html: false, linkify: true });

/** Speech reads prose and link labels; executable/code content stays in the transcript. */
export function voiceSpeechText(content: string): string {
  const source = sanitizeAssistantMessageContent(content)
    .replace(/<@[^>|]+\|([^>]+)>/g, "$1")
    .replace(/<@[^>]+>/g, "someone");
  const render = (tokens: Token[]): string => {
    let result = "";
    let autoLink = false;
    for (const token of tokens) {
      if (token.type === "link_open" && token.info === "auto") {
        autoLink = true;
        result += "link";
      } else if (token.type === "link_close") autoLink = false;
      else if (autoLink) continue;
      else if (token.type === "fence" || token.type === "code_block")
        result += " Code omitted. ";
      else if (token.type === "inline") result += render(token.children ?? []);
      else if (["text", "code_inline", "image"].includes(token.type))
        result += token.content;
      else if (
        [
          "softbreak",
          "hardbreak",
          "paragraph_close",
          "heading_close",
          "tr_close",
          "list_item_close",
        ].includes(token.type)
      )
        result += "\n";
      else if (token.type === "th_close" || token.type === "td_close")
        result += ", ";
    }
    return result;
  };
  return render(markdown.parse(source, {}))
    .replace(/(^|[\s(])@([\p{L}\p{N}_-]+)/gu, "$1$2")
    .replace(/,\s*(?=\n|$)/g, "")
    .replace(/\s+/g, " ")
    .trim();
}
