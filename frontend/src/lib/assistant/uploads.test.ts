import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { capture } from "@/lib/telemetry";
import { transitionAssistantIdentity } from "./identity";
import { uploadFile } from "./uploads";

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

  it.each([403, 500])("reports HTTP %s once with bounded metadata and trace correlation", async (status) => {
    const pending = expect(uploadFile(scope, file, new AbortController().signal, vi.fn())).rejects.toThrow("private response");
    const xhr = UploadRequest.latest;
    const trace = "76dd13d5-e7cf-4c63-a33b-26036d2a495d";
    xhr.responseHeaders.set("X-NyxID-Trace-Id", trace);
    xhr.respond(status, JSON.stringify({ message: "private response" }));
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
