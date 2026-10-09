import { beforeEach, describe, expect, it, vi } from "vitest";
import { ZodError } from "zod";

const { invokeMock, listenMock, unlistenMock } = vi.hoisted(() => ({
  invokeMock: vi.fn(),
  listenMock: vi.fn(),
  unlistenMock: vi.fn(),
}));

vi.mock("@tauri-apps/api/core", () => ({
  invoke: invokeMock,
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: listenMock,
}));

import { TauriCompanionRuntime } from "./tauri-runtime";

const authorizingView = {
  state: "authorizing",
  userCode: "ABCD-1234",
  verificationUrl: "https://nyx.chrono-ai.fun/login/device?user_code=ABCD-1234",
  expiresAt: "2026-10-09T10:10:00Z",
} as const;

let nyxidEventHandler: ((event: { payload: unknown }) => void) | undefined;

describe("TauriCompanionRuntime NyxID contract", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    nyxidEventHandler = undefined;
    listenMock.mockImplementation(
      async (
        _eventName: string,
        handler: (event: { payload: unknown }) => void,
      ) => {
        nyxidEventHandler = handler;
        return unlistenMock;
      },
    );
  });

  it.each([
    ["nyxidStatus", "nyxid_status"],
    ["startNyxidLogin", "start_nyxid_login"],
    ["cancelNyxidLogin", "cancel_nyxid_login"],
    ["refreshNyxidCapabilities", "refresh_nyxid_capabilities"],
    ["logoutNyxid", "logout_nyxid"],
  ] as const)(
    "invokes %s with the exact %s command",
    async (method, command) => {
      invokeMock.mockResolvedValueOnce({ state: "signed_out" });
      const runtime = new TauriCompanionRuntime();

      await expect(runtime[method]()).resolves.toEqual({ state: "signed_out" });

      expect(invokeMock).toHaveBeenCalledOnce();
      expect(invokeMock).toHaveBeenCalledWith(command);
    },
  );

  it("parses valid command results", async () => {
    invokeMock.mockResolvedValueOnce(authorizingView);
    const runtime = new TauriCompanionRuntime();

    await expect(runtime.startNyxidLogin()).resolves.toEqual(authorizingView);
  });

  it("rejects malformed command results", async () => {
    invokeMock.mockResolvedValueOnce({
      ...authorizingView,
      deviceCode: "must-not-cross-the-runtime-boundary",
    });
    const runtime = new TauriCompanionRuntime();

    await expect(runtime.startNyxidLogin()).rejects.toBeInstanceOf(ZodError);
  });

  it("forwards valid events, suppresses malformed events, and returns cleanup", async () => {
    const listener = vi.fn();
    const runtime = new TauriCompanionRuntime();

    const unlisten = await runtime.onNyxidChanged(listener);

    expect(listenMock).toHaveBeenCalledOnce();
    expect(listenMock).toHaveBeenCalledWith(
      "companion://nyxid-changed",
      expect.any(Function),
    );
    expect(nyxidEventHandler).toBeTypeOf("function");

    nyxidEventHandler?.({ payload: authorizingView });
    expect(listener).toHaveBeenCalledOnce();
    expect(listener).toHaveBeenCalledWith(authorizingView);

    nyxidEventHandler?.({
      payload: {
        ...authorizingView,
        refreshToken: "must-not-cross-the-runtime-boundary",
      },
    });
    expect(listener).toHaveBeenCalledOnce();

    await unlisten();
    expect(unlistenMock).toHaveBeenCalledOnce();
  });
});
