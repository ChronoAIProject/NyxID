import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { capture } from "@/lib/telemetry";
import { transitionAssistantIdentity } from "./identity";
import { isSupportedAttachment, uploadFile } from "./uploads";
import { assistantUploadNameHeader, sanitizeAssistantUploadName } from "../../../../shared/assistantUploadFilename";

vi.mock("@/lib/telemetry", async (importOriginal) => ({
  ...await importOriginal<typeof import("@/lib/telemetry")>(),
  capture: vi.fn(),
}));

class UploadRequest {
  static latest: UploadRequest;
  status = 0;
  responseText = "";
  responseHeaders = new Headers();
  upload = { onprogress: null };
  onerror: (() => void) | null = null;
  ontimeout: (() => void) | null = null;
  onabort: (() => void) | null = null;
  onload: (() => void) | null = null;
  onloadend: (() => void) | null = null;
  constructor() { UploadRequest.latest = this; }
  open = vi.fn();
  send = vi.fn();
  setRequestHeader = vi.fn();
  getResponseHeader(name: string) { return this.responseHeaders.get(name); }
  abort() { this.onabort?.(); this.onloadend?.(); }
  respond(status: number, body: string) {
    this.status = status;
    this.responseText = body;
    this.onload?.();
    this.onloadend?.();
  }
}

const file = new File(["private image bytes"], "private-photo.png", { type: "image/png" });
const scope = { kind: "conversations", id: "private-conversation" } as const;

describe("production attachment upload error telemetry", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.stubGlobal("XMLHttpRequest", UploadRequest);
    globalThis.__nyxidAssistantHttpMock = undefined;
    transitionAssistantIdentity("upload-owner");
  });
  afterEach(() => vi.unstubAllGlobals());

  it.each([
    ["a".repeat(117) + ".v1.csv ", "application/vnd.ms-excel", "a".repeat(116) + ".csv"],
    ["a".repeat(117) + ".v1.csv. .🔥", "application/vnd.ms-excel", "a".repeat(116) + ".csv"],
    ["Invoice.json. .", "application/json", "Invoice.json"],
    ["Invoice.json. .🔥", "application/json", "Invoice.json"],
    ["🧾.PDF.", "application/pdf", undefined],
    ["🧾.PDF. .🔥", "application/pdf", undefined],
    [".gitignore. .🔥", "text/plain", "gitignore"],
    ["attachment. .", "application/pdf", "attachment"],
  ])("normalizes trailing padding before classification and the actual request: %s", async (name, type, expected) => {
    const selected = new File(["original bytes"], name, { type });
    expect(isSupportedAttachment(selected)).toBe(true);
    expect(sanitizeAssistantUploadName(name)).toBe(expected);
    if (expected !== undefined) {
      expect(sanitizeAssistantUploadName(expected)).toBe(expected);
      expect(assistantUploadNameHeader(expected)).toBe(assistantUploadNameHeader(name));
    }
    const pending = uploadFile(scope, selected, new AbortController().signal, vi.fn());
    const xhr = UploadRequest.latest;
    const sent = xhr.setRequestHeader.mock.calls.find(([key]) => key.toLowerCase() === "x-attachment-name");
    expect(sent?.[1]).toBe(expected === undefined ? undefined : encodeURIComponent(expected));
    expect(xhr.send).toHaveBeenCalledExactlyOnceWith(selected);
    xhr.respond(201, JSON.stringify({ id: "saved", content_type: "text/plain", size: selected.size, label: expected ?? "attachment.pdf" }));
    await pending;
  });

  it.each([
    ["a".repeat(118) + ".pdf", "a".repeat(116) + ".pdf"],
    ["a".repeat(121) + ".pdf", "a".repeat(116) + ".pdf"],
    [" ".repeat(120) + "a.pdf", "a.pdf"],
    [" .".repeat(120) + "a.pdf", "a.pdf"],
    ["attachment", "attachment"],
    ["🧾.PDF", undefined],
    ["📷.jpg", undefined],
    ["..", undefined],
  ])("preflight and raw upload preserve the server filename contract for %s", async (name, expected) => {
    const selected = new File(["original bytes"], name);
    expect(isSupportedAttachment(selected)).toBe(true);
    const pending = uploadFile(scope, selected, new AbortController().signal, vi.fn());
    const xhr = UploadRequest.latest;
    const sent = xhr.setRequestHeader.mock.calls.find(([key]) => key.toLowerCase() === "x-attachment-name");
    expect(sent?.[1]).toBe(expected === undefined ? undefined : encodeURIComponent(expected));
    expect(xhr.send).toHaveBeenCalledExactlyOnceWith(selected);
    xhr.respond(201, JSON.stringify({ id: "saved", content_type: "application/pdf", size: selected.size, label: expected ?? "attachment.pdf" }));
    await pending;
  });

  it.each(["报".repeat(118) + ".pdf", "𠮷".repeat(118) + ".pdf"])("fits encoded Unicode names into the real header cap: %s", async (name) => {
    const selected = new File(["original bytes"], name);
    expect(isSupportedAttachment(selected)).toBe(true);
    const pending = uploadFile(scope, selected, new AbortController().signal, vi.fn());
    const xhr = UploadRequest.latest;
    const sent = xhr.setRequestHeader.mock.calls.find(([key]) => key.toLowerCase() === "x-attachment-name")?.[1];
    expect(sent).toBeDefined();
    expect(sent.length).toBeLessThanOrEqual(1024);
    expect(decodeURIComponent(sent)).toMatch(/\.pdf$/);
    expect([...decodeURIComponent(sent)].length).toBeLessThanOrEqual(120);
    xhr.respond(201, JSON.stringify({ id: "saved", content_type: "application/pdf", size: selected.size, label: decodeURIComponent(sent) }));
    await pending;
  });

  it.each(["🧾.PDF", "attachment", "报".repeat(118) + ".pdf"])("uses the same optional name header in the preview transport: %s", async (name) => {
    const mock = vi.fn().mockResolvedValue(new Response(JSON.stringify({ id: "preview", content_type: "application/pdf", size: 3, label: "preview.pdf" }), { status: 201 }));
    globalThis.__nyxidAssistantHttpMock = mock;
    try {
      await uploadFile(scope, new File(["pdf"], name), new AbortController().signal, vi.fn());
      const headers = new Headers(mock.mock.calls[0]?.[0].init.headers);
      expect(headers.get("x-attachment-name")).toBe(assistantUploadNameHeader(name) ?? null);
    } finally { globalThis.__nyxidAssistantHttpMock = undefined; }
  });

  it.each([403, 500])("reports HTTP %s once with bounded metadata and trace correlation", async (status) => {
    const pending = expect(uploadFile(scope, file, new AbortController().signal, vi.fn())).rejects.toThrow("private response");
    const xhr = UploadRequest.latest;
    const trace = "76dd13d5-e7cf-4c63-a33b-26036d2a495d";
    xhr.responseHeaders.set("X-NyxID-Trace-Id", trace);
    xhr.respond(status, JSON.stringify({ error: "request_failed", error_code: 1000, message: "private response" }));
    // A second callback must not emit another event for this upload.
    xhr.onerror?.();
    await pending;
    expect(capture).toHaveBeenCalledExactlyOnceWith({ name: "nyxbot.error", props: expect.objectContaining({
      error_code: status === 403 ? "NYXBOT_HTTP_FORBIDDEN" : "NYXBOT_HTTP_SERVER_ERROR",
      engine: "nyxagent", stage: "request", route: "/assistant/nyxagent/:resource", http_status: status, trace_id: trace,
    }) });
    const serialized = JSON.stringify(vi.mocked(capture).mock.calls);
    expect(serialized).not.toContain("private");
    expect(serialized).not.toContain("image bytes");
  });

  it.each([
    ["network", "NYXBOT_NETWORK_ERROR"],
    ["timeout", "NYXBOT_START_TIMEOUT"],
    ["json", "NYXBOT_PROTOCOL_ERROR"],
    ["schema", "NYXBOT_PROTOCOL_ERROR"],
  ])("reports %s failures", async (kind, code) => {
    const pending = expect(uploadFile(scope, file, new AbortController().signal, vi.fn())).rejects.toThrow();
    const xhr = UploadRequest.latest;
    if (kind === "network") xhr.onerror?.();
    else if (kind === "timeout") xhr.ontimeout?.();
    else xhr.respond(200, kind === "json" ? "private malformed body" : "{}");
    await pending;
    expect(capture).toHaveBeenCalledExactlyOnceWith({ name: "nyxbot.error", props: expect.objectContaining({ error_code: code }) });
  });

  it.each([false, true])("excludes account transitions including a switch back (ABA: %s)", async (aba) => {
    const pending = expect(uploadFile(scope, file, new AbortController().signal, vi.fn())).rejects.toThrow();
    transitionAssistantIdentity("another-owner");
    if (aba) transitionAssistantIdentity("upload-owner");
    UploadRequest.latest.respond(403, "{}");
    await pending;
    expect(capture).not.toHaveBeenCalled();
  });

  it("excludes cancellation even if a later failure callback fires", async () => {
    const controller = new AbortController();
    const pending = expect(uploadFile(scope, file, controller.signal, vi.fn())).rejects.toThrow("Upload cancelled.");
    controller.abort();
    UploadRequest.latest.onerror?.();
    await pending;
    expect(capture).not.toHaveBeenCalled();
  });

  it.each([[503, "<html>nginx</html>"], [502, "private proxy body"], [504, ""], [503, '{"error":']])(
    "sanitizes an upload HTTP %s error without replay", async (status, body) => {
      const pending = uploadFile(scope, file, new AbortController().signal, vi.fn());
      const xhr = UploadRequest.latest;
      xhr.responseHeaders.set("x-request-id", "request-2275");
      xhr.respond(status as number, body as string);
      await expect(pending).rejects.toMatchObject({ status, requestId: "request-2275", message: "NyxID is temporarily unavailable. Try again in a moment." });
      expect(xhr.send).toHaveBeenCalledTimes(1);
    });

  it("preserves a successful upload and emits no error", async () => {
    const progress = vi.fn();
    const pending = uploadFile(scope, file, new AbortController().signal, progress);
    const attachment = { id: "attachment", content_type: "image/png", size: file.size, label: "private-photo.png" };
    UploadRequest.latest.respond(201, JSON.stringify(attachment));
    await expect(pending).resolves.toEqual(attachment);
    expect(capture).not.toHaveBeenCalled();
  });

  it("does not count file-size validation as a request failure", async () => {
    await expect(uploadFile(scope, new File([], "empty.png"), new AbortController().signal, vi.fn())).rejects.toThrow("Choose a file");
    expect(capture).not.toHaveBeenCalled();
  });
});
