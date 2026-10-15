import { Node, type JSONContent } from "@tiptap/core";
import { Plugin, TextSelection } from "@tiptap/pm/state";
import type { Node as ProseMirrorNode } from "@tiptap/pm/model";
import { mentionTokens } from "@/lib/assistant/nyxbot-mentions";

export interface ComposerToken {
  id: string;
  handle: string;
  label: string;
  kind: "agent" | "image" | "file" | "pasted-text";
}
export const MAX_COMPOSER_LENGTH = 32_768;
const leafText = (node: ProseMirrorNode) =>
  node.type.name === "composerMention"
    ? `@${String(node.attrs.handle)}`
    : node.type.name === "hardBreak"
      ? "\n"
      : "";
export function editorText(
  doc: ProseMirrorNode,
  end = doc.content.size,
): string {
  return doc.textBetween(0, end, "\n", leafText);
}
/** Map plain-text draft offsets to editor positions, treating each pill as one unit. */
export function editorPosition(doc: ProseMirrorNode, offset: number): number {
  let length = 0;
  let result = 1;
  doc.forEach((paragraph, blockPosition, index) => {
    if (index) {
      length += 1;
      if (length <= offset) result = blockPosition + 1;
    }
    paragraph.forEach((node, position) => {
      const size = node.isText ? node.text!.length : leafText(node).length;
      if (length < offset)
        result =
          blockPosition +
          1 +
          position +
          (node.isText ? Math.min(size, offset - length) : node.nodeSize);
      length += size;
    });
  });
  return Math.min(result, doc.content.size - 1);
}
export function tokenSegments(text: string, tokens: readonly ComposerToken[]) {
  const known = new Map(
    tokens.map((token) => [token.handle.toLowerCase(), token]),
  );
  const parts: { text: string; token?: ComposerToken }[] = [];
  let end = 0;
  for (const match of mentionTokens(text)) {
    const token = known.get(match.name.toLowerCase());
    if (!token) continue;
    if (match.start > end) parts.push({ text: text.slice(end, match.start) });
    end = match.start + match.name.length + 1;
    parts.push({
      text: text.slice(match.start, end),
      token: { ...token, handle: match.name },
    });
  }
  if (end < text.length) parts.push({ text: text.slice(end) });
  return parts;
}
export function editorDocument(
  text: string,
  tokens: readonly ComposerToken[],
): JSONContent {
  const content: JSONContent[] = [];
  for (const part of tokenSegments(text, tokens)) {
    if (part.token)
      content.push({ type: "composerMention", attrs: part.token });
    else
      part.text.split("\n").forEach((line, index) => {
        if (index) content.push({ type: "hardBreak" });
        if (line) content.push({ type: "text", text: line });
      });
  }
  return { type: "doc", content: [{ type: "paragraph", content }] };
}

export function composerMention(getTokens: () => readonly ComposerToken[]) {
  return Node.create({
    name: "composerMention",
    group: "inline",
    inline: true,
    atom: true,
    selectable: true,
    addAttributes: () => ({
      id: { default: "" },
      handle: { default: "" },
      label: { default: "" },
      kind: { default: "agent" },
    }),
    // Pasted HTML cannot mint trusted mention nodes. Only our known candidates can.
    parseHTML: () => [],
    renderHTML: ({ node }) => [
      "span",
      {
        "data-composer-mention": node.attrs.kind,
        "data-mention-handle": node.attrs.handle,
        class: "composer-mention",
        contenteditable: "false",
        title: `@${String(node.attrs.handle)}`,
      },
      `${node.attrs.kind === "agent" ? "@" : ""}${String(node.attrs.label)}`,
    ],
    renderText: ({ node }) => `@${String(node.attrs.handle)}`,
    addKeyboardShortcuts() {
      const remove = (backward: boolean) => {
        const { state, view } = this.editor;
        if (!state.selection.empty) return false;
        const { $from } = state.selection;
        const node = backward ? $from.nodeBefore : $from.nodeAfter;
        if (node?.type.name !== this.name) return false;
        const start = backward ? $from.pos - node.nodeSize : $from.pos;
        view.dispatch(state.tr.delete(start, start + node.nodeSize));
        return true;
      };
      return { Backspace: () => remove(true), Delete: () => remove(false) };
    },
    addProseMirrorPlugins() {
      return [
        new Plugin({
          filterTransaction: (transaction, state) =>
            !transaction.docChanged ||
            editorText(transaction.doc).length <=
              Math.max(MAX_COMPOSER_LENGTH, editorText(state.doc).length),
          appendTransaction: (transactions, _old, state) => {
            if (
              !transactions.some((transaction) => transaction.docChanged) ||
              this.editor.view.composing
            )
              return null;
            const tokens = getTokens();
            const known = new Map(
              tokens.map((token) => [token.handle.toLowerCase(), token]),
            );
            const changes: {
              from: number;
              to: number;
              token?: ComposerToken;
            }[] = [];
            state.doc.descendants((node, pos) => {
              if (
                node.type.name === "composerMention" &&
                !tokens.some((token) => token.id === node.attrs.id)
              ) {
                changes.push({ from: pos, to: pos + node.nodeSize });
              }
              if (!node.isText) return;
              for (const match of mentionTokens(node.text!)) {
                const end = match.start + match.name.length + 1;
                const token = known.get(match.name.toLowerCase());
                // Keep the active query editable until a delimiter completes it.
                if (token && end < node.text!.length)
                  changes.push({
                    from: pos + match.start,
                    to: pos + end,
                    token: { ...token, handle: match.name },
                  });
              }
            });
            if (!changes.length) return null;
            const tr = state.tr;
            for (const change of changes.reverse()) {
              if (change.token)
                tr.replaceWith(
                  change.from,
                  change.to,
                  state.schema.nodes.composerMention!.create(change.token),
                );
              else tr.delete(change.from, change.to);
            }
            return tr;
          },
        }),
      ];
    },
  });
}

export function selectEditorOffsets(
  doc: ProseMirrorNode,
  start: number,
  end = start,
) {
  return TextSelection.create(
    doc,
    editorPosition(doc, start),
    editorPosition(doc, end),
  );
}
