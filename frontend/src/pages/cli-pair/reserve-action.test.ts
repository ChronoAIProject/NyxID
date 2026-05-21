import { beforeEach, describe, expect, it, vi } from "vitest";
import { ApiError } from "@/lib/api-client";
import {
  reservePairingAction,
  rewindPairingAction,
  withRewindOnError,
} from "./reserve-action";

const { mockPost } = vi.hoisted(() => ({
  mockPost: vi.fn(),
}));

vi.mock("@/lib/api-client", () => ({
  api: {
    post: mockPost,
  },
  // Mirror the real ApiError contract used by reserve-action.ts: a
  // `status` field is the only thing the branch logic reads.
  ApiError: class ApiError extends Error {
    status: number;
    errorCode: number;
    constructor(
      status: number,
      response: { message: string; error_code: number },
    ) {
      super(response.message);
      this.status = status;
      this.errorCode = response.error_code;
    }
  },
}));

function apiError(status: number, message = "boom"): ApiError {
  return new ApiError(status, { message, error_code: 0, error: "x" });
}

beforeEach(() => {
  mockPost.mockReset();
});

describe("reservePairingAction", () => {
  it("posts to /cli-pairings/<encoded id>/reserve-action and resolves on 2xx", async () => {
    mockPost.mockResolvedValueOnce({ ok: true });

    await expect(
      reservePairingAction("pair abc/1"),
    ).resolves.toBeUndefined();

    expect(mockPost).toHaveBeenCalledWith(
      "/cli-pairings/pair%20abc%2F1/reserve-action",
      {},
    );
  });

  it.each([409, 404])(
    "throws the stale-tab message on %i ApiError",
    async (status) => {
      mockPost.mockRejectedValueOnce(apiError(status));

      await expect(reservePairingAction("pair-1")).rejects.toThrow(
        /already completed or started in another tab/i,
      );
    },
  );

  it("throws the generic 'Couldn't reserve' message (with detail) on a non-409/404 ApiError", async () => {
    mockPost.mockRejectedValueOnce(apiError(500, "server exploded"));

    await expect(reservePairingAction("pair-1")).rejects.toThrow(
      /Couldn't reserve this pairing with NyxID \(server exploded\)/,
    );
  });

  it("throws the generic 'Couldn't reserve' message on a plain network error", async () => {
    mockPost.mockRejectedValueOnce(new Error("network down"));

    await expect(reservePairingAction("pair-1")).rejects.toThrow(
      /Couldn't reserve this pairing with NyxID \(network down\)/,
    );
  });
});

describe("rewindPairingAction", () => {
  it("posts to /cli-pairings/<encoded id>/rewind-action", async () => {
    mockPost.mockResolvedValueOnce({ ok: true });

    await rewindPairingAction("pair abc/1");

    expect(mockPost).toHaveBeenCalledWith(
      "/cli-pairings/pair%20abc%2F1/rewind-action",
      {},
    );
  });

  it("swallows errors and never throws", async () => {
    mockPost.mockRejectedValueOnce(new Error("rewind failed"));

    await expect(rewindPairingAction("pair-1")).resolves.toBeUndefined();
  });
});

describe("withRewindOnError", () => {
  it("returns the run() value and does NOT rewind on success", async () => {
    const run = vi.fn().mockResolvedValue("minted-key");

    const result = await withRewindOnError("pair-1", run);

    expect(result).toBe("minted-key");
    expect(run).toHaveBeenCalledTimes(1);
    // No rewind POST on the happy path.
    expect(mockPost).not.toHaveBeenCalled();
  });

  it("rewinds THEN rethrows on a 4xx ApiError", async () => {
    const err = apiError(409);
    const run = vi.fn().mockRejectedValue(err);
    mockPost.mockResolvedValueOnce({ ok: true });

    await expect(withRewindOnError("pair-1", run)).rejects.toBe(err);

    // Rewind fired before the rethrow — proves the latch was undone.
    expect(mockPost).toHaveBeenCalledWith(
      "/cli-pairings/pair-1/rewind-action",
      {},
    );
  });

  it("rethrows a 5xx ApiError WITHOUT rewinding (ambiguous — may have committed)", async () => {
    const err = apiError(500);
    const run = vi.fn().mockRejectedValue(err);

    await expect(withRewindOnError("pair-1", run)).rejects.toBe(err);

    expect(mockPost).not.toHaveBeenCalled();
  });

  it("rethrows a plain network Error WITHOUT rewinding", async () => {
    const err = new Error("connection reset");
    const run = vi.fn().mockRejectedValue(err);

    await expect(withRewindOnError("pair-1", run)).rejects.toBe(err);

    expect(mockPost).not.toHaveBeenCalled();
  });
});
