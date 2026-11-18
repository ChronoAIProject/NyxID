import { flushSync } from "react-dom";
import { createRoot } from "react-dom/client";
import type { InlineMention } from "@/lib/assistant/group-mentions";
import { MarkdownContent } from "@/components/assistant/blocks/text-block";

export type MessageCopyFormat = "plain" | "markdown";

const BLOCK_ELEMENTS = new Set([
  "DIV", "SECTION", "P", "PRE", "UL", "OL", "H1", "H2", "H3", "H4", "H5", "H6",
  "BLOCKQUOTE", "TABLE", "THEAD", "TBODY", "TR", "HR",
]);

/** Keep block boundaries separate from content, so code whitespace stays intact. */
function flowParts(node: HTMLElement): Node[][] {
  const parts: Node[][] = [];
  let inline: Node[] = [];
  const flush = () => {
    if (inline.some((child) => child.textContent?.trim())) parts.push(inline);
    inline = [];
  };
  for (const child of node.childNodes) {
    if (child instanceof HTMLElement && BLOCK_ELEMENTS.has(child.tagName)) {
      flush();
      parts.push([child]);
    } else {
      if (!inline.length && child.nodeType === Node.TEXT_NODE && !child.textContent?.trim() && child.textContent?.includes("\n")) continue;
      inline.push(child);
    }
  }
  flush();
  return parts;
}

/** Extract readable text from our own safe rendering, never from the chat DOM. */
function readableText(node: Node, depth = 0): string {
  if (node.nodeType === Node.TEXT_NODE) return node.textContent ?? "";
  if (!(node instanceof HTMLElement)) return "";
  const tag = node.tagName;
  if (tag === "BR") return "\n";
  if (tag === "PRE") return node.textContent ?? "";
  if (tag === "HR") return "";
  if (tag === "UL" || tag === "OL") {
    return [...node.children].map((item, index) => {
      const marker = tag === "OL" ? `${Number(node.getAttribute("start") ?? 1) + index}. ` : "• ";
      const indent = "  ".repeat(depth);
      let first = true;
      const parts = flowParts(item as HTMLElement);
      return parts.map((part, partIndex) => {
        const child = part[0];
        if (child instanceof HTMLElement && ["UL", "OL"].includes(child.tagName)) {
          return readableText(child, depth + 1);
        }
        let text = part.map((child) => readableText(child, depth)).join("");
        const next = parts[partIndex + 1]?.[0];
        if (next instanceof HTMLElement && ["UL", "OL"].includes(next.tagName) && part.at(-1)?.nodeType === Node.TEXT_NODE) {
          text = text.replace(/\n$/, "");
        }
        const prefix = first ? indent + marker : indent + " ".repeat(marker.length);
        first = false;
        // Continuation lines stay within this list item, including code lines.
        return prefix + text.replace(/\n(?=.)/g, `\n${indent}${" ".repeat(marker.length)}`);
      }).join("\n");
    }).join("\n");
  }
  if (tag === "TABLE" || tag === "THEAD" || tag === "TBODY") {
    return [...node.children].map((child) => readableText(child, depth)).join("\n");
  }
  if (tag === "TR") return [...node.children].map((cell) => readableText(cell, depth)).join("\t");
  if (["DIV", "SECTION", "BLOCKQUOTE", "LI", "TD", "TH"].includes(tag)) {
    return flowParts(node).map((part) => part.map((child) => readableText(child, depth)).join("")).join("\n\n");
  }
  const content = [...node.childNodes].map((child) => readableText(child, depth)).join("");
  return tag === "SUP" ? `[${content}]` : content;
}

export function messageClipboardContent(text: string, mentions?: readonly InlineMention[]): { text: string; html: string } {
  const host = document.createElement("div");
  const root = createRoot(host);
  let body: HTMLElement;
  // MarkdownContent owns the renderer's sanitizer, URL policy, literal HTML,
  // images-as-links, mentions and CJK parsing. Render only on Copy, without
  // streaming hooks/chat chrome or adding react-dom/server to the app bundle.
  try {
    flushSync(() => root.render(<MarkdownContent text={text} mentions={mentions} />));
    body = host.cloneNode(true) as HTMLElement;
  } finally {
    flushSync(() => root.unmount());
  }
  body.querySelectorAll('[hidden], [aria-hidden="true"], .sr-only').forEach((element) => element.remove());
  // Fragment links in a copied message cannot target the original chat DOM.
  // Keep their labels, and omit footnote back-navigation controls entirely.
  body.querySelectorAll('a[href^="#"]').forEach((link) => {
    if (link.getAttribute("href")?.startsWith("#user-content-fnref-")) {
      const previous = link.previousSibling;
      if (previous?.nodeType === Node.TEXT_NODE) previous.textContent = previous.textContent?.replace(/[ \t]+$/, "") ?? "";
      link.remove();
    }
    else link.replaceWith(...link.childNodes);
  });
  body.querySelectorAll("input").forEach((input) => {
    input.replaceWith(document.createTextNode(input.checked ? "[x]" : "[ ]"));
  });
  // Paragraphs render with white-space: pre-wrap, including their inline code.
  // Preserve spaces after removing app classes; br also works in paste targets
  // that discard inline styles. Headings and tight lists keep their own rules.
  for (const paragraph of body.querySelectorAll("p")) {
    paragraph.style.whiteSpace = "pre-wrap";
    const walker = document.createTreeWalker(paragraph, NodeFilter.SHOW_TEXT);
    const nodes: Text[] = [];
    while (walker.nextNode()) nodes.push(walker.currentNode as Text);
    for (const node of nodes) {
      if (node.parentElement?.closest("code")) continue;
      const value = node.previousSibling instanceof HTMLBRElement ? node.data.replace(/^\n/, "") : node.data;
      node.replaceWith(...value.split("\n").flatMap((line, index) => index === 0
        ? [document.createTextNode(line)]
        : [document.createElement("br"), document.createTextNode(line)]));
    }
  }
  for (const element of body.querySelectorAll("*")) {
    for (const attribute of [...element.attributes]) {
      if (!["href", "rel", "target", "start", "style", "colspan", "rowspan"].includes(attribute.name)) {
        element.removeAttribute(attribute.name);
      }
    }
  }
  return { text: readableText(body), html: body.innerHTML };
}

export async function copyMessage(text: string, format: MessageCopyFormat, mentions?: readonly InlineMention[]): Promise<void> {
  const clipboard = navigator.clipboard;
  if (!clipboard) throw new Error("Clipboard unavailable");
  if (format === "plain") return clipboard.writeText(text);
  const content = messageClipboardContent(text, mentions);
  if (typeof clipboard.write === "function" && typeof ClipboardItem !== "undefined" &&
      (typeof ClipboardItem.supports !== "function" || ClipboardItem.supports("text/html"))) {
    try {
      await clipboard.write([new ClipboardItem({
        "text/plain": new Blob([content.text], { type: "text/plain" }),
        "text/html": new Blob([content.html], { type: "text/html" }),
      })]);
      return;
    } catch (error) {
      // Unsupported rich formats may still allow text. A permission refusal
      // remains a failed copy and must not be reported as success.
      if (!(error instanceof DOMException) || error.name !== "NotSupportedError") throw error;
    }
  }
  await clipboard.writeText(content.text);
}
