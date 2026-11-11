import assert from "node:assert/strict";
import test from "node:test";
import {
  activeMention,
  attachmentKind,
  insertMention,
  mentionCandidates,
  resolveAttachmentMentions,
  stripAttachmentMention,
  type AttachmentMention,
} from "./assistantMentions";

const files: AttachmentMention[] = [
  { handle: "image-1", label: "chart.png", kind: "image" },
  { handle: "file-1", label: "report.pdf", kind: "file" },
  { handle: "text-1", label: "pasted-text-1.md", kind: "pasted-text" },
];

function at(text: string, caret = text.length) {
  return activeMention(text, { start: caret, end: caret });
}

test("native autocomplete starts after whitespace and matches Web ASCII handles", () => {
  assert.deepEqual(at("@"), { start: 0, end: 1, query: "" });
  assert.deepEqual(at("See @file-1"), { start: 4, end: 11, query: "file-1" });
  assert.deepEqual(at("See\n@im"), { start: 4, end: 7, query: "im" });
  for (const text of [
    "mail@file",
    "(@file",
    "@agent_name",
    "@文件",
    "@file.",
  ]) {
    assert.equal(at(text), null, text);
  }
});

test("a caret in an existing handle replaces the suffix rather than duplicating it", () => {
  const text = "Compare @image-1 with chart";
  const mention = at(text, "Compare @im".length)!;
  assert.deepEqual(mention, { start: 8, end: 16, query: "im" });
  assert.deepEqual(insertMention(text, mention, "image-2"), {
    text: "Compare @image-2 with chart",
    selection: { start: 17, end: 17 },
  });
});

test("insertion preserves punctuation, newlines, and an additional intentional space", () => {
  for (const rest of [", next", "\nnext", "  next"]) {
    const text = `@fi${rest}`;
    const mention = at(text, 3)!;
    const result = insertMention(text, mention, "file-1");
    assert.equal(result.text, `@file-1 ${rest.replace(/^ /, "")}`);
    assert.deepEqual(result.selection, { start: 8, end: 8 });
  }
});

test("native selection ranges and invalid cursor offsets never open autocomplete", () => {
  const text = "@file-1";
  for (const selection of [
    { start: 0, end: 7 },
    { start: 2, end: 5 },
    { start: 5, end: 2 },
    { start: -1, end: -1 },
    { start: 8, end: 8 },
    { start: 2.5, end: 2.5 },
    { start: NaN, end: NaN },
  ])
    assert.equal(activeMention(text, selection), null);
});

test("Chinese IME and emoji retain UTF-16 selection positions and surrounding text", () => {
  assert.equal(at("请@im"), null);
  assert.equal(at("请 @图片"), null);
  const text = "😀 请 @im 后面";
  const caret = "😀 请 @im".length;
  const mention = at(text, caret)!;
  assert.deepEqual(mention, { start: 5, end: 8, query: "im" });
  assert.deepEqual(insertMention(text, mention, "image-1"), {
    text: "😀 请 @image-1 后面",
    selection: { start: 14, end: 14 },
  });
});

test("autocomplete searches handle and label case-insensitively, prefix before substring", () => {
  const options = [
    { handle: "file-1", label: "Yearly-report.pdf", kind: "file" },
    { handle: "image-1", label: "Report chart.png", kind: "image" },
    { handle: "reporter", label: "News specialist", kind: "agent" },
    { handle: "file-2", label: "notes.txt", kind: "file" },
  ];
  assert.deepEqual(mentionCandidates(options, "REPort"), [
    options[1],
    options[2],
    options[0],
  ]);
  assert.deepEqual(mentionCandidates(options, "file"), [
    options[0],
    options[3],
  ]);
  assert.deepEqual(mentionCandidates(options, ""), options);
  assert.deepEqual(mentionCandidates(options, "missing"), []);
});

test("attachment descriptions match Web without adding routing fields", () => {
  assert.equal(
    resolveAttachmentMentions(
      "Compare @image-1 with @file-1. Also @text-1",
      files,
    ),
    'Compare image "chart.png" with file "report.pdf". Also pasted text "pasted-text-1.md"',
  );
});

test("all @ signs in uploaded labels are removed to prevent agent mention injection", () => {
  assert.equal(
    resolveAttachmentMentions("@file-1", [
      {
        handle: "file-1",
        label: "notes @researcher and @@admin.txt",
        kind: "file",
      },
    ]),
    'file "notes researcher and admin.txt"',
  );
});

test("replacement is one pass even when a filename contains another known handle", () => {
  assert.equal(
    resolveAttachmentMentions("Read @file-1 then @image-1", [
      { handle: "file-1", label: "refer to @image-1.pdf", kind: "file" },
      files[0]!,
    ]),
    'Read file "refer to image-1.pdf" then image "chart.png"',
  );
});

test("attachment resolution follows Web punctuation and email word boundaries", () => {
  const text =
    "(@file-1), [@image-1] @file-1. mail@file-1 a.@file-1 x_@file-1 x-@file-1";
  assert.equal(
    resolveAttachmentMentions(text, files),
    '(file "report.pdf"), [image "chart.png"] file "report.pdf". mail@file-1 a.@file-1 x_@file-1 x-@file-1',
  );
});

test("unknown, differently cased, or longer handles remain unchanged", () => {
  const text =
    "@file-1 @file-10 @file-1x @file-1_name @file-1-more @FILE-1 @other";
  assert.equal(
    resolveAttachmentMentions(text, files),
    'file "report.pdf" @file-10 @file-1x @file-1_name @file-1-more @FILE-1 @other',
  );
  assert.equal(resolveAttachmentMentions(text, []), text);
});

test("known overlapping handles each resolve to the correct file", () => {
  assert.equal(
    resolveAttachmentMentions("@file-10 @file-1", [
      files[1]!,
      { handle: "file-10", label: "ten.pdf", kind: "file" },
    ]),
    'file "ten.pdf" file "report.pdf"',
  );
});

test("handle metacharacters are escaped rather than interpreted as regex", () => {
  const special = {
    handle: "file(1)",
    label: "one.pdf",
    kind: "file",
  } as const;
  assert.equal(
    resolveAttachmentMentions("@file(1) @file1", [special]),
    'file "one.pdf" @file1',
  );
  assert.equal(
    stripAttachmentMention("@file(1) @file1", special.handle),
    "@file1",
  );
});

test("deleting a chip removes its repeated references and one following space only", () => {
  assert.equal(
    stripAttachmentMention(
      "@file-1 then (@file-1), @file-10 and @image-1",
      "file-1",
    ),
    "then (), @file-10 and @image-1",
  );
  assert.equal(
    stripAttachmentMention("@file-1  next\n@file-1\nend", "file-1"),
    " next\n\nend",
  );
  assert.equal(
    stripAttachmentMention("mail@file-1 x.@file-1 @file-1_more", "file-1"),
    "mail@file-1 x.@file-1 @file-1_more",
  );
});

test("image handle kinds match the Web MIME allowlist", () => {
  for (const type of ["image/png", "image/jpeg", "image/gif", "image/webp"]) {
    assert.equal(attachmentKind({ type }), "image");
  }
  for (const type of ["image/svg+xml", "application/pdf", "text/plain", ""]) {
    assert.equal(attachmentKind({ type }), "file");
  }
});
