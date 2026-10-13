import { describe, expect, it } from "vitest";
import {
  attachmentKind,
  draftWithoutHandles,
  isLongPaste,
  pastedTextFile,
  resolveAttachmentMentions,
  stripAttachmentMention,
} from "./attachment-mentions";

const mentions = [
  { handle: "image-1", label: "chart.png", kind: "image" as const },
  { handle: "image-10", label: "other.png", kind: "image" as const },
  { handle: "file-1", label: "report.pdf", kind: "file" as const },
  { handle: "pasted-text-1", label: "pasted-text-1.md", kind: "pasted-text" as const },
];

describe("attachment mentions", () => {
  it("names each mentioned file in the prompt and leaves look-alikes alone", () => {
    expect(
      resolveAttachmentMentions(
        "Compare @image-1 with @image-10, see (@file-1) and @pasted-text-1. Mail me@image-1.",
        mentions,
      ),
    ).toBe(
      'Compare image "chart.png" with image "other.png", see (file "report.pdf") and pasted text "pasted-text-1.md". Mail me@image-1.',
    );
  });

  it("never lets a file name add an @mention", () => {
    expect(
      resolveAttachmentMentions("Summarize @file-1", [
        { handle: "file-1", label: "@researcher notes.txt", kind: "file" },
      ]),
    ).toBe('Summarize file "researcher notes.txt"');
  });

  it("saves drafts with pasted text inline and other files described, never a dangling handle", () => {
    expect(
      draftWithoutHandles("Compare @pasted-text-1 with @image-1", [
        { handle: "pasted-text-1", label: "pasted-text-1.txt", kind: "pasted-text", text: "line 1\nline 2" },
        { handle: "image-1", label: "chart.png", kind: "image" },
      ]),
    ).toBe('Compare line 1\nline 2 with image "chart.png"');
    expect(
      draftWithoutHandles("@pasted-text-1", [
        { handle: "pasted-text-1", label: "p.txt", kind: "pasted-text", text: "x".repeat(40_000) },
      ]),
    ).toHaveLength(32_768);
  });

  it("leaves a handle with no attachment as typed", () => {
    expect(resolveAttachmentMentions("see @image-3", mentions)).toBe("see @image-3");
  });

  it("removes a removed attachment's references", () => {
    expect(stripAttachmentMention("@image-1 and @image-10 here @image-1", "image-1")).toBe(
      "and @image-10 here ",
    );
  });

  it("attaches pastes longer than six lines or a wall of text", () => {
    expect(isLongPaste("1\n2\n3\n4\n5\n6")).toBe(false);
    expect(isLongPaste("1\n2\n3\n4\n5\n6\n7")).toBe(true);
    expect(isLongPaste("x".repeat(1_501))).toBe(true);
  });

  it("keeps pasted Markdown as Markdown and other text as plain text", async () => {
    const markdown = pastedTextFile("# Title\n\n- item", 2);
    expect([markdown.name, markdown.type]).toEqual(["pasted-text-2.md", "text/markdown"]);
    expect(await markdown.text()).toBe("# Title\n\n- item");
    const plain = pastedTextFile("just words\nmore words", 3);
    expect([plain.name, plain.type]).toEqual(["pasted-text-3.txt", "text/plain"]);
  });

  it("tells images from other files", () => {
    expect(attachmentKind({ type: "image/webp" })).toBe("image");
    expect(attachmentKind({ type: "application/pdf" })).toBe("file");
  });
});
