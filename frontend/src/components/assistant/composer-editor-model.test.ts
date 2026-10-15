import { describe, expect, it } from "vitest";
import { getSchema } from "@tiptap/core";
import Document from "@tiptap/extension-document";
import Paragraph from "@tiptap/extension-paragraph";
import Text from "@tiptap/extension-text";
import HardBreak from "@tiptap/extension-hard-break";
import {
  composerMention,
  editorDocument,
  editorPosition,
  editorText,
  tokenSegments,
  type ComposerToken,
} from "./composer-editor-model";
const tokens: ComposerToken[] = [
  { id: "agent", handle: "researcher", label: "Researcher", kind: "agent" },
  {
    id: "attachment:file-1",
    handle: "file-1",
    label: "Design notes.md",
    kind: "file",
  },
];
const schema = getSchema([
  Document,
  Paragraph,
  Text,
  HardBreak,
  composerMention(() => tokens),
]);
describe("composer editor references", () => {
  it("serializes readable pills to the existing handles and preserves newlines", () => {
    const text = "Ask @researcher about @file-1\nThen summarize.\n";
    const doc = schema.nodeFromJSON(editorDocument(text, tokens));
    expect(editorText(doc)).toBe(text);
    expect(doc.firstChild!.child(1).attrs.label).toBe("Researcher");
    for (const offset of [0, 4, 15, 22, 29, text.length]) {
      expect(editorText(doc, editorPosition(doc, offset)).length).toBe(offset);
    }
  });
  it("does not turn emails or unknown handles into trusted references", () => {
    expect(
      tokenSegments("mail@researcher @unknown @researcher-more", tokens).some(
        (part) => part.token,
      ),
    ).toBe(false);
    expect(
      tokenSegments("(@RESEARCHER), @file-1", tokens)
        .filter((part) => part.token)
        .map((part) => part.token?.id),
    ).toEqual(["agent", "attachment:file-1"]);
  });
  it("maps caret positions across native paragraphs and empty lines", () => {
    const doc = schema.nodeFromJSON({
      type: "doc",
      content: [
        { type: "paragraph", content: [{ type: "text", text: "one" }] },
        { type: "paragraph" },
        { type: "paragraph", content: [{ type: "text", text: "three" }] },
      ],
    });
    expect(editorText(doc)).toBe("one\n\nthree");
    for (let offset = 0; offset <= 10; offset++)
      expect(editorText(doc, editorPosition(doc, offset)).length).toBe(offset);
  });
  it("keeps hostile filenames as text attributes", () => {
    const doc = schema.nodeFromJSON(
      editorDocument("@file-1", [
        { ...tokens[1]!, label: '<img src=x onerror="bad()">' },
      ]),
    );
    expect(editorText(doc)).toBe("@file-1");
    expect(doc.firstChild!.firstChild!.attrs.label).toBe(
      '<img src=x onerror="bad()">',
    );
  });
});
