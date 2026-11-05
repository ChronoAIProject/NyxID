import { afterEach, beforeEach, expect, it, vi } from "vitest";
import {
  acquireGeneratedAttachment,
  clearGeneratedAttachmentCache,
} from "./generated-attachment-cache";
import { assistantHttp } from "./assistant-http";
import { transitionAssistantIdentity } from "./identity";
vi.mock("./assistant-http", () => ({ assistantHttp: vi.fn() }));
beforeEach(() => {
  vi.clearAllMocks();
  vi.stubGlobal("URL", {
    createObjectURL: vi.fn(() => "blob:private"),
    revokeObjectURL: vi.fn(),
  });
  vi.mocked(assistantHttp).mockImplementation(
    async () => new Response("private bytes"),
  );
});
afterEach(() => {
  clearGeneratedAttachmentCache();
  vi.useRealTimers();
  vi.unstubAllGlobals();
});
it("shares concurrent fetches and retains a completed blob across list remounts", async () => {
  const first = acquireGeneratedAttachment("/private/file");
  const second = acquireGeneratedAttachment("/private/file");
  expect(await first.ready).toBe(await second.ready);
  expect(assistantHttp).toHaveBeenCalledTimes(1);
  await Promise.resolve();
  first.release();
  second.release();
  const remount = acquireGeneratedAttachment("/private/file");
  expect(await remount.ready).toBe("blob:private");
  expect(assistantHttp).toHaveBeenCalledTimes(1);
  remount.release();
  clearGeneratedAttachmentCache();
  expect(URL.revokeObjectURL).toHaveBeenCalledTimes(1);
});
it("pins active URLs within the byte budget and evicts a released file", async () => {
  const leases = Array.from({ length: 4 }, (_, n) =>
    acquireGeneratedAttachment(`/file/${n}`),
  );
  await Promise.all(leases.map((lease) => lease.ready));
  await Promise.resolve();
  const queued = acquireGeneratedAttachment("/file/5");
  expect(assistantHttp).toHaveBeenCalledTimes(4);
  leases[0]!.release();
  expect(await queued.ready).toBe("blob:private");
  expect(assistantHttp).toHaveBeenCalledTimes(5);
  expect(URL.revokeObjectURL).toHaveBeenCalledTimes(1);
  leases.forEach((lease) => lease.release());
  queued.release();
});
it("rejects oversized bodies and cancels an unmounted pending fetch", async () => {
  vi.mocked(assistantHttp).mockResolvedValueOnce(
    new Response("x", {
      headers: { "content-length": String(9 * 1024 * 1024) },
    }),
  );
  const oversize = acquireGeneratedAttachment("/oversize");
  await expect(oversize.ready).rejects.toThrow(/limit/);
  oversize.release();
  vi.mocked(assistantHttp).mockImplementationOnce(
    (_path, options) =>
      new Promise((_resolve, reject) => {
        options?.signal?.addEventListener("abort", () =>
          reject(new DOMException("aborted", "AbortError")),
        );
      }),
  );
  const pending = acquireGeneratedAttachment("/pending");
  const failed = expect(pending.ready).rejects.toThrow(/released/);
  pending.release();
  await failed;
});
it("revokes on expiry, idle eviction and account change", async () => {
  vi.useFakeTimers();
  const expiry = acquireGeneratedAttachment(
    "/expiry",
    new Date(Date.now() + 100).toISOString(),
  );
  await expiry.ready;
  await vi.advanceTimersByTimeAsync(100);
  expect(URL.revokeObjectURL).toHaveBeenCalledTimes(1);
  expiry.release();
  const idle = acquireGeneratedAttachment("/idle");
  await idle.ready;
  await Promise.resolve();
  idle.release();
  await vi.advanceTimersByTimeAsync(120_000);
  expect(URL.revokeObjectURL).toHaveBeenCalledTimes(2);
  const account = acquireGeneratedAttachment("/account");
  await account.ready;
  transitionAssistantIdentity("another-owner");
  expect(URL.revokeObjectURL).toHaveBeenCalledTimes(3);
  account.release();
});

it("fits eight known small files together and rejects bytes exceeding their reservation", async () => {
  const leases = Array.from({ length: 8 }, (_, n) =>
    acquireGeneratedAttachment(`/small/${n}`, undefined, 32),
  );
  await Promise.all(leases.map((lease) => lease.ready));
  expect(assistantHttp).toHaveBeenCalledTimes(8);
  const understated = acquireGeneratedAttachment("/understated", undefined, 1);
  await expect(understated.ready).rejects.toThrow(/limit/);
  leases.forEach((lease) => lease.release());
  understated.release();
});

it("holds an aborted fetch's byte reservation until its body has settled", async () => {
  const pinned = Array.from({ length: 3 }, (_, n) =>
    acquireGeneratedAttachment(`/pinned/${n}`),
  );
  await Promise.all(pinned.map((lease) => lease.ready));
  await Promise.resolve();
  let finishCancellation!: () => void;
  vi.mocked(assistantHttp).mockImplementationOnce(
    () =>
      new Promise((_resolve, reject) => {
        finishCancellation = () => reject(new DOMException("aborted", "AbortError"));
      }),
  );
  const abandoned = acquireGeneratedAttachment("/abandoned");
  const rejected = expect(abandoned.ready).rejects.toThrow(/released/);
  abandoned.release();
  await rejected;
  const queued = acquireGeneratedAttachment("/after-abandoned");
  const callsBeforeCancellationSettled = vi.mocked(assistantHttp).mock.calls.length;
  finishCancellation();
  await queued.ready;
  pinned.forEach((lease) => lease.release());
  queued.release();
  expect(callsBeforeCancellationSettled).toBe(4);
  expect(assistantHttp).toHaveBeenCalledTimes(5);
});
