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
  transitionAssistantIdentity(null);
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

it("keeps a pinned 30-day URL live past the timer cap and revokes at its deadline", async () => {
  vi.useFakeTimers();
  vi.setSystemTime(new Date("2026-01-01T00:00:00Z"));
  const lifetime = 30 * 24 * 60 * 60 * 1000;
  const timerCap = 2_147_483_647;
  const lease = acquireGeneratedAttachment(
    "/long-lived",
    new Date(Date.now() + lifetime).toISOString(),
  );
  expect(await lease.ready).toBe("blob:private");

  // Only two timer hops cover the full retention period.
  await vi.advanceTimersByTimeAsync(timerCap);
  expect(URL.revokeObjectURL).not.toHaveBeenCalled();
  expect(vi.getTimerCount()).toBe(1);
  await vi.advanceTimersByTimeAsync(lifetime - timerCap - 1);
  expect(URL.revokeObjectURL).not.toHaveBeenCalled();
  await vi.advanceTimersByTimeAsync(1);
  expect(URL.revokeObjectURL).toHaveBeenCalledExactlyOnceWith("blob:private");
  lease.release();
  expect(vi.getTimerCount()).toBe(0);
});

it.each([
  { lifetime: 50, idle: true },
  { lifetime: 1_000, idle: true },
  { lifetime: 50, idle: false },
])(
  "reauthenticates changed expiry ($lifetime ms, idle=$idle) and retires the old href",
  async ({ lifetime, idle }) => {
    vi.useFakeTimers();
    vi.mocked(URL.createObjectURL).mockReturnValueOnce("blob:old").mockReturnValueOnce("blob:refreshed");
    const now = Date.now();
    const first = acquireGeneratedAttachment(
      "/refreshed-policy",
      new Date(now + 100).toISOString(),
    );
    await first.ready;
    await vi.advanceTimersByTimeAsync(0);
    if (idle) first.release();
    const refreshed = acquireGeneratedAttachment("/refreshed-policy", new Date(now + lifetime).toISOString());
    expect(await refreshed.ready).toBe("blob:refreshed");
    expect(assistantHttp).toHaveBeenCalledTimes(2);
    expect(URL.revokeObjectURL).toHaveBeenCalledExactlyOnceWith("blob:old");
    first.release();
    await vi.advanceTimersByTimeAsync(lifetime - 1);
    expect(URL.revokeObjectURL).not.toHaveBeenCalledWith("blob:refreshed");
    await vi.advanceTimersByTimeAsync(1);
    expect(URL.revokeObjectURL).toHaveBeenCalledWith("blob:refreshed");
    expect(URL.revokeObjectURL).toHaveBeenCalledTimes(2);
    refreshed.release();
    expect(vi.getTimerCount()).toBe(0);
  },
);

it("cancels idle eviction on remount and starts a fresh idle lease on final release", async () => {
  vi.useFakeTimers();
  const first = acquireGeneratedAttachment("/remounted");
  await first.ready;
  await vi.advanceTimersByTimeAsync(0);
  first.release();
  await vi.advanceTimersByTimeAsync(119_999);
  const remount = acquireGeneratedAttachment("/remounted");
  expect(await remount.ready).toBe("blob:private");
  await vi.advanceTimersByTimeAsync(120_000);
  expect(URL.revokeObjectURL).not.toHaveBeenCalled();
  expect(assistantHttp).toHaveBeenCalledTimes(1);
  remount.release();
  remount.release();
  await vi.advanceTimersByTimeAsync(119_999);
  expect(URL.revokeObjectURL).not.toHaveBeenCalled();
  await vi.advanceTimersByTimeAsync(1);
  expect(URL.revokeObjectURL).toHaveBeenCalledTimes(1);
  expect(vi.getTimerCount()).toBe(0);
});

it("does not reuse expired bytes when the expiry timer is delayed", async () => {
  vi.useFakeTimers();
  const deadline = Date.now() + 100;
  const first = acquireGeneratedAttachment(
    "/delayed-expiry",
    new Date(deadline).toISOString(),
  );
  await first.ready;
  // Move wall time without running timers, as with a suspended/background tab.
  vi.setSystemTime(deadline);
  const second = acquireGeneratedAttachment(
    "/delayed-expiry",
    new Date(deadline).toISOString(),
  );
  await expect(second.ready).rejects.toMatchObject({ name: "AbortError" });
  expect(assistantHttp).toHaveBeenCalledTimes(1);
  expect(URL.revokeObjectURL).toHaveBeenCalledTimes(1);
  first.release();
  second.release();
  expect(vi.getTimerCount()).toBe(0);
});

it.each(["expiry", "delayed timer", "identity"])(
  "discards pending bytes after %s",
  async (cause) => {
    vi.useFakeTimers();
    let finish!: (response: Response) => void;
    vi.mocked(assistantHttp).mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          finish = resolve;
        }),
    );
    const deadline = Date.now() + 100;
    const lease = acquireGeneratedAttachment(
      "/expires-pending",
      new Date(deadline).toISOString(),
    );
    const rejected = expect(lease.ready).rejects.toMatchObject({
      name: "AbortError",
    });
    if (cause === "expiry") await vi.advanceTimersByTimeAsync(100);
    else if (cause === "delayed timer") vi.setSystemTime(deadline);
    else transitionAssistantIdentity("new-owner");
    finish(new Response("late bytes"));
    await rejected;
    await vi.advanceTimersByTimeAsync(0);
    expect(vi.mocked(assistantHttp).mock.calls[0]![1]?.signal?.aborted).toBe(true);
    expect(URL.createObjectURL).not.toHaveBeenCalled();
    expect(URL.revokeObjectURL).not.toHaveBeenCalled();
    lease.release();
    expect(vi.getTimerCount()).toBe(0);
  },
);

it("clears identity timers and isolates a replacement entry from stale releases", async () => {
  vi.useFakeTimers();
  vi.mocked(URL.createObjectURL)
    .mockReturnValueOnce("blob:old")
    .mockReturnValueOnce("blob:new");
  const deadline = new Date(Date.now() + 30 * 24 * 60 * 60 * 1000).toISOString();
  const old = acquireGeneratedAttachment("/identity-file", deadline);
  await old.ready;
  transitionAssistantIdentity("replacement-owner");
  expect(URL.revokeObjectURL).toHaveBeenCalledExactlyOnceWith("blob:old");
  expect(vi.getTimerCount()).toBe(0);
  const replacement = acquireGeneratedAttachment("/identity-file", deadline);
  expect(await replacement.ready).toBe("blob:new");
  old.release();
  old.release();
  expect(vi.getTimerCount()).toBe(1);
  expect(vi.mocked(assistantHttp).mock.calls[0]![1]?.signal?.aborted).toBe(true);
  expect(vi.mocked(assistantHttp).mock.calls[1]![1]?.signal?.aborted).toBe(false);
  clearGeneratedAttachmentCache();
  replacement.release();
  expect(URL.revokeObjectURL).toHaveBeenCalledTimes(2);
  expect(URL.revokeObjectURL).toHaveBeenLastCalledWith("blob:new");
  expect(vi.getTimerCount()).toBe(0);
});

it("does not fall back to cached bytes when refreshed policy fails authorization", async () => {
  vi.useFakeTimers();
  const now = Date.now();
  const first = acquireGeneratedAttachment("/reauthorize", new Date(now + 100).toISOString());
  await first.ready;
  await vi.advanceTimersByTimeAsync(0);
  first.release();
  vi.mocked(assistantHttp).mockRejectedValueOnce(new Error("Access denied"));
  const refreshed = acquireGeneratedAttachment("/reauthorize", new Date(now + 1_000).toISOString());
  await expect(refreshed.ready).rejects.toThrow("Access denied");
  expect(assistantHttp).toHaveBeenCalledTimes(2);
  expect(URL.createObjectURL).toHaveBeenCalledTimes(1);
  refreshed.release();
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
