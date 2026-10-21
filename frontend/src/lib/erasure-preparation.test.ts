import { afterEach, describe, expect, it, vi } from "vitest";
import { ApiError } from "@/lib/api-client";
import { withErasurePreparation } from "./erasure-preparation";
import { useAuthStore } from "@/stores/auth-store";

const preparing = () => new ApiError(409, { error: "erasure_preparing", error_code: 12801, message: "Preparing erasure" });
afterEach(() => { vi.useRealTimers(); vi.restoreAllMocks(); });
describe("erasure preparation backoff", () => {
  it("backs off on preparation and preserves the accepted response", async () => {
    vi.useFakeTimers();
    const accepted = { status: "accepted" };
    const request = vi.fn().mockRejectedValueOnce(preparing()).mockRejectedValueOnce(preparing()).mockResolvedValue(accepted);
    const result = withErasurePreparation(request);
    expect(request).toHaveBeenCalledTimes(1);
    await vi.advanceTimersByTimeAsync(499);
    expect(request).toHaveBeenCalledTimes(1);
    await vi.advanceTimersByTimeAsync(1);
    expect(request).toHaveBeenCalledTimes(2);
    await vi.advanceTimersByTimeAsync(1000);
    expect(await result).toBe(accepted);
  });
  it("does not retry other conflicts", async () => {
    const error = new ApiError(409, { error: "conflict", error_code: 1004, message: "Shared client dependency" });
    const request = vi.fn().mockRejectedValue(error);
    await expect(withErasurePreparation(request)).rejects.toBe(error);
    expect(request).toHaveBeenCalledTimes(1);
  });
  it("does not issue a retry after the account changes", async () => {
    vi.useFakeTimers();
    const original = useAuthStore.getState();
    let switched = false;
    vi.spyOn(useAuthStore, "getState").mockImplementation(() => switched ? { ...original, user: { ...original.user, id: "successor-account" } } as typeof original : original);
    const request = vi.fn().mockRejectedValue(preparing());
    const result = expect(withErasurePreparation(request)).rejects.toThrow("signed-in account changed");
    switched = true;
    await vi.advanceTimersByTimeAsync(500);
    await result;
    expect(request).toHaveBeenCalledTimes(1);
  });
  it("bounds retries and explains how to resume preparation", async () => {
    vi.useFakeTimers();
    const error = preparing();
    const request = vi.fn().mockRejectedValue(error);
    const result = expect(withErasurePreparation(request)).rejects.toThrow("Account deletion is still being prepared; try again in a minute.");
    await vi.runAllTimersAsync();
    await result;
    expect(request).toHaveBeenCalledTimes(10);
  });
});
