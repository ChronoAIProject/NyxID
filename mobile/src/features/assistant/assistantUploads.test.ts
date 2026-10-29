import assert from "node:assert/strict";
import test from "node:test";
import {
  ATTACHMENT_SIZE_MESSAGE,
  MAX_ASSISTANT_ATTACHMENT_BYTES,
  UNSUPPORTED_ATTACHMENT_MESSAGE,
  assistantAttachmentPath,
  attachmentFileError,
  attachmentUploadError,
  attachmentUploadHeaders,
  isSupportedAttachment,
  safeAttachmentName,
  withAttachmentUploadDeadline,
} from "./assistantUploads";

test("the picker accepts the same binary and text families as Web", () => {
  const files = [
    { name: "image.bin", mimeType: "image/png" },
    { name: "image.bin", mimeType: "IMAGE/JPEG; charset=binary" },
    { name: "image.bin", mimeType: "image/gif" },
    { name: "image.bin", mimeType: "image/webp" },
    { name: "report.bin", mimeType: "application/pdf" },
    {
      name: "document.bin",
      mimeType:
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
    },
    { name: "notes", mimeType: "text/plain" },
    { name: "notes.TXT", mimeType: "text/plain" },
    { name: "notes.text", mimeType: "text/plain" },
    { name: "notes.MD", mimeType: "text/markdown" },
    { name: "notes.markdown", mimeType: "text/plain" },
    { name: "data.json", mimeType: "application/json" },
    { name: "data.csv", mimeType: "text/csv" },
    { name: "data.CSV", mimeType: "application/vnd.ms-excel" },
    { name: "data.csv", mimeType: "application/csv" },
  ];
  for (const file of files)
    assert.equal(isSupportedAttachment(file), true, file.name);
});

test("unknown MIME guesses preserve suffix based and extensionless support", () => {
  for (const name of [
    "README",
    "note.txt",
    "note.MD",
    "note.markdown",
    "data.csv",
    "data.JSON",
    "scan.PNG",
    "scan.jpg",
    "scan.jpeg",
    "scan.gif",
    "scan.webp",
    "report.PDF",
    "report.docx",
  ]) {
    assert.equal(isSupportedAttachment({ name }), true, name);
    assert.equal(
      isSupportedAttachment({ name, mimeType: "application/octet-stream" }),
      true,
      name,
    );
  }
});

test("unsupported files remain actionable errors before any upload", () => {
  for (const file of [
    { name: "data.zip", mimeType: "application/zip" },
    { name: "report.doc", mimeType: "application/msword" },
    { name: "script.js", mimeType: "text/javascript" },
    { name: "image.svg", mimeType: "image/svg+xml" },
    { name: "movie.mp4", mimeType: "video/mp4" },
    { name: "data.json", mimeType: "application/zip" },
    { name: "data.json", mimeType: "application/vnd.ms-excel" },
    { name: "archive.zip", mimeType: "application/octet-stream" },
  ]) {
    assert.equal(attachmentFileError(file), UNSUPPORTED_ATTACHMENT_MESSAGE);
  }
});

test("accepted upload sizes are integral and bounded from 1 byte to 20 MiB", () => {
  const file = { name: "note.txt", mimeType: "text/plain" };
  assert.equal(attachmentFileError({ ...file, size: 1 }), null);
  assert.equal(
    attachmentFileError({ ...file, size: MAX_ASSISTANT_ATTACHMENT_BYTES }),
    null,
  );
  for (const size of [
    0,
    -1,
    0.5,
    NaN,
    Infinity,
    MAX_ASSISTANT_ATTACHMENT_BYTES + 1,
  ]) {
    assert.equal(
      attachmentFileError({ ...file, size }),
      ATTACHMENT_SIZE_MESSAGE,
    );
  }
});

test("actual bytes decide size admission when picker metadata is missing or stale", () => {
  const file = { name: "note.txt", size: null };
  assert.equal(attachmentFileError(file), null);
  assert.equal(attachmentFileError(file, 0), ATTACHMENT_SIZE_MESSAGE);
  assert.equal(
    attachmentFileError(
      { ...file, size: 1 },
      MAX_ASSISTANT_ATTACHMENT_BYTES + 1,
    ),
    ATTACHMENT_SIZE_MESSAGE,
  );
  assert.equal(attachmentFileError({ ...file, size: 0 }, 10), null);
});

test("display names match server path, control character and Unicode normalization", () => {
  assert.equal(safeAttachmentName("../../my\nfile.pdf"), "myfile.pdf");
  assert.equal(safeAttachmentName("C:\\private\\notes (2).MD"), "notes (2).MD");
  assert.equal(safeAttachmentName(" ...报告🙂 (2).json... "), "报告 (2).json");
  assert.equal(safeAttachmentName(".."), "attachment");
  assert.equal(safeAttachmentName("\n\r\0"), "attachment");
  assert.equal([...safeAttachmentName("𠮷".repeat(121))].length, 120);
  assert.equal(safeAttachmentName("note.tx🔥t"), "note.txt");
  assert.equal(isSupportedAttachment({ name: "note.tx🔥t" }), true);
});

test("attachment paths keep conversation/group and attachment identifiers scoped", () => {
  assert.equal(
    assistantAttachmentPath({ kind: "conversations", id: "nyxa-one" }),
    "/assistant/nyxagent/conversations/nyxa-one/attachments",
  );
  assert.equal(
    assistantAttachmentPath({ kind: "groups", id: "group /?" }, "file /?"),
    "/assistant/nyxagent/groups/group%20%2F%3F/attachments/file%20%2F%3F",
  );
  assert.throws(
    () => assistantAttachmentPath({ kind: "conversations" }),
    /Choose a conversation first/,
  );
  assert.throws(
    () => assistantAttachmentPath({ kind: "conversations", id: " " }),
    /Choose a conversation first/,
  );
  assert.throws(
    () =>
      assistantAttachmentPath({ kind: "conversations", id: "nyxa-one" }, ""),
    /Choose an attachment first/,
  );
});

test("upload headers send a percent encoded safe label with raw byte content type", () => {
  assert.deepEqual(attachmentUploadHeaders("../../报告 (2).json"), {
    "Content-Type": "application/octet-stream",
    "X-Attachment-Name": "%E6%8A%A5%E5%91%8A%20(2).json",
  });
  const long = attachmentUploadHeaders("报".repeat(115) + ".json");
  assert.ok(long["X-Attachment-Name"]!.length <= 1024);
  assert.ok(decodeURIComponent(long["X-Attachment-Name"]!).endsWith(".json"));
});

test("upload failures preserve actionable server guidance and normalize malformed files", () => {
  assert.equal(
    attachmentUploadError(new Error("Malformed or unsupported attachment")),
    UNSUPPORTED_ATTACHMENT_MESSAGE,
  );
  assert.equal(
    attachmentUploadError(
      new Error("Upload processing is busy. Try again shortly."),
    ),
    "Upload processing is busy. Try again shortly.",
  );
  assert.equal(
    attachmentUploadError(
      Object.assign(new Error("cancelled"), { name: "AbortError" }),
    ),
    "Upload cancelled.",
  );
  assert.equal(
    attachmentUploadError(undefined),
    "Upload failed. Please try again.",
  );
});

test("an already cancelled selection never starts its upload", async () => {
  const controller = new AbortController();
  controller.abort();
  let requests = 0;
  await assert.rejects(
    withAttachmentUploadDeadline(async () => {
      requests++;
      return "ready";
    }, controller.signal),
    { name: "AbortError" },
  );
  assert.equal(requests, 0);
});

test("removing a pending selection cancels its transport even if it is stuck", async () => {
  const controller = new AbortController();
  let uploadSignal: AbortSignal | undefined;
  let requests = 0;
  const upload = withAttachmentUploadDeadline((signal) => {
    requests++;
    uploadSignal = signal;
    return new Promise<never>(() => undefined);
  }, controller.signal);
  controller.abort();
  await assert.rejects(upload, { name: "AbortError" });
  assert.equal(uploadSignal?.aborted, true);
  assert.equal(requests, 1);
});

test("the upload deadline aborts work and never retries a potentially accepted POST", async () => {
  let uploadSignal: AbortSignal | undefined;
  let requests = 0;
  await assert.rejects(
    withAttachmentUploadDeadline(
      (signal) => {
        requests++;
        uploadSignal = signal;
        return new Promise<never>(() => undefined);
      },
      undefined,
      5,
    ),
    /Upload timed out. Try a smaller file/,
  );
  assert.equal(uploadSignal?.aborted, true);
  assert.equal(requests, 1);
});

test("completed uploads release cancellation listeners", async () => {
  const controller = new AbortController();
  let uploadSignal: AbortSignal | undefined;
  const accepted = await withAttachmentUploadDeadline(async (signal) => {
    uploadSignal = signal;
    return { id: "stored-upload" };
  }, controller.signal);
  assert.deepEqual(accepted, { id: "stored-upload" });
  controller.abort();
  assert.equal(uploadSignal?.aborted, false);
});
