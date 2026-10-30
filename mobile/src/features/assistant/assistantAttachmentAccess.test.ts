import assert from "node:assert/strict";
import test from "node:test";
import { ApiError } from "../../lib/api/ApiError";
import type { AssistantAttachment } from "./assistantTypes";
import { MAX_ASSISTANT_ATTACHMENT_BYTES } from "./assistantUploads";
import {
  ATTACHMENT_EXPIRED_MESSAGE,
  ATTACHMENT_UNAVAILABLE_MESSAGE,
  assertReadableAttachment,
  attachmentAccessError,
  attachmentCacheName,
  isPreviewableAttachment,
} from "./assistantAttachmentAccess";

const document: AssistantAttachment = {
  id: "attachment-id",
  content_type: "text/plain",
  label: "notes",
  size: 10,
  origin: "user_upload",
};

test("owner reads support the full 20 MiB upload limit", () => {
  const file = { ...document, size: MAX_ASSISTANT_ATTACHMENT_BYTES };
  assert.doesNotThrow(() => assertReadableAttachment(file, file.size));
  for (const size of [0, -1, NaN, Infinity, 1.2, file.size + 1])
    assert.throws(() => assertReadableAttachment({ ...file, size }));
  assert.throws(() => assertReadableAttachment(document, document.size + 1));
  assert.throws(() => assertReadableAttachment(document, 0));
});

test("expired metadata and HTTP retention responses give the same guidance", () => {
  assert.throws(
    () => assertReadableAttachment({ ...document, expired: true }),
    new Error(ATTACHMENT_EXPIRED_MESSAGE),
  );
  for (const [statusCode, errorCode] of [
    [410, 0],
    [400, 12101],
  ])
    assert.equal(
      attachmentAccessError(
        new ApiError({
          statusCode: statusCode!,
          errorCode: errorCode!,
          errorKey: "expired",
          message: "internal",
        }),
      ),
      ATTACHMENT_EXPIRED_MESSAGE,
    );
});

test("private cache uses a safe leaf filename, with an extension for native apps", () => {
  assert.equal(attachmentCacheName(document), "notes.txt");
  assert.equal(
    attachmentCacheName({ ...document, label: "../../private/notes.txt" }),
    "notes.txt",
  );
  assert.equal(
    attachmentCacheName({
      ...document,
      label: "..\\private\\report.pdf",
      content_type: "application/pdf",
    }),
    "report.pdf",
  );
  assert.equal(
    attachmentCacheName({ ...document, label: "..." }),
    "attachment.txt",
  );
  assert.equal(
    attachmentCacheName({
      ...document,
      label: "report",
      content_type: "application/pdf",
    }),
    "report.pdf",
  );
});

test("image input unavailable is still owner-previewable and other types open as documents", () => {
  for (const content_type of [
    "image/png",
    "image/jpeg",
    "image/gif",
    "image/webp",
  ])
    assert.equal(
      isPreviewableAttachment({
        ...document,
        content_type,
        image_input: "unavailable",
      }),
      true,
    );
  for (const content_type of ["application/pdf", "text/plain", "image/svg+xml"])
    assert.equal(isPreviewableAttachment({ ...document, content_type }), false);
});

test("native cache suffix follows the verified content type rather than the original name", () => {
  for (const [label, content_type, expected] of [
    ["photo.bin", "image/png", "photo.png"],
    ["photo.jpg", "image/png", "photo.png"],
    ["report.bin", "application/pdf", "report.pdf"],
    ["report.PDF", "application/pdf", "report.pdf"],
    ["paragraphs.bin", "text/plain", "paragraphs.txt"],
    ["notes.bin", "text/markdown", "notes.md"],
    [
      "report.bin",
      "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
      "report.docx",
    ],
  ]) {
    const attachment = {
      ...document,
      label: label!,
      content_type: content_type!,
    };
    assert.equal(attachmentCacheName(attachment), expected);
    assert.equal(attachment.label, label);
  }
});

test("native errors do not expose local paths or filenames", () => {
  assert.equal(
    attachmentAccessError(new Error("Failed to write /private/user-file.txt")),
    ATTACHMENT_UNAVAILABLE_MESSAGE,
  );
  assert.equal(
    attachmentAccessError(undefined),
    ATTACHMENT_UNAVAILABLE_MESSAGE,
  );
});
