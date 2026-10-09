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

import {
  NYXID_CHAT_ADMISSION_UNKNOWN_MESSAGE,
  NyxIdChatSendError,
} from "./chat";
import { TauriCompanionRuntime } from "./tauri-runtime";

const authorizingView = {
  state: "authorizing",
  userCode: "ABCD-1234",
  verificationUrl: "https://nyx.chrono-ai.fun/login/device?user_code=ABCD-1234",
  expiresAt: "2026-10-09T10:10:00Z",
} as const;

const conversationId = "nyxa-0123456789abcdef0123456789abcdef";
const requestId = "123e4567-e89b-42d3-a456-426614174000";

let nyxidEventHandler: ((event: { payload: unknown }) => void) | undefined;
let windowDragEventHandler: ((event: { payload: unknown }) => void) | undefined;

describe("TauriCompanionRuntime NyxID contract", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    nyxidEventHandler = undefined;
    windowDragEventHandler = undefined;
    listenMock.mockImplementation(
      async (
        eventName: string,
        handler: (event: { payload: unknown }) => void,
      ) => {
        if (eventName === "companion://window-drag") {
          windowDragEventHandler = handler;
        } else {
          nyxidEventHandler = handler;
        }
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

  it("forwards only strict native window drag events", async () => {
    const listener = vi.fn();
    const runtime = new TauriCompanionRuntime();

    const unlisten = await runtime.onWindowDrag(listener);

    expect(listenMock).toHaveBeenCalledWith(
      "companion://window-drag",
      expect.any(Function),
    );
    windowDragEventHandler?.({
      payload: { phase: "moving", direction: "right" },
    });
    windowDragEventHandler?.({
      payload: { phase: "settled", direction: null },
    });
    expect(listener).toHaveBeenNthCalledWith(1, {
      phase: "moving",
      direction: "right",
    });
    expect(listener).toHaveBeenNthCalledWith(2, {
      phase: "settled",
      direction: null,
    });

    windowDragEventHandler?.({
      payload: {
        phase: "settled",
        direction: null,
        accessToken: "must-not-cross-runtime",
      },
    });
    expect(listener).toHaveBeenCalledTimes(2);

    await unlisten();
    expect(unlistenMock).toHaveBeenCalledOnce();
  });

  it("invokes the narrow chat send, recovery, history, and stop commands", async () => {
    const completed = {
      kind: "completed",
      requestId,
      conversationId,
      status: "completed",
      error: null,
    } as const;
    const history = {
      conversation: { id: conversationId, activeTurn: false },
      messages: [
        {
          id: "message-1",
          seq: 1,
          turnId: "turn-1",
          role: "assistant",
          text: "处理完成",
          status: "completed",
          errorCode: null,
        },
      ],
    } as const;
    invokeMock
      .mockResolvedValueOnce(completed)
      .mockResolvedValueOnce({ requestId, history })
      .mockResolvedValueOnce(history)
      .mockResolvedValueOnce(undefined);
    const runtime = new TauriCompanionRuntime();

    await expect(
      runtime.sendNyxIdChat({ requestId, text: "帮我处理" }),
    ).resolves.toEqual(completed);
    await expect(runtime.recoverNyxIdChat()).resolves.toEqual({
      requestId,
      history,
    });
    await expect(runtime.nyxIdChatHistory(conversationId)).resolves.toEqual(
      history,
    );
    await expect(
      runtime.stopNyxIdChat(conversationId),
    ).resolves.toBeUndefined();

    expect(invokeMock).toHaveBeenNthCalledWith(1, "send_nyxid_chat", {
      request: { requestId, text: "帮我处理" },
    });
    expect(invokeMock).toHaveBeenNthCalledWith(2, "recover_nyxid_chat");
    expect(invokeMock).toHaveBeenNthCalledWith(3, "nyxid_chat_history", {
      conversationId,
    });
    expect(invokeMock).toHaveBeenNthCalledWith(4, "nyxid_chat_stop", {
      conversationId,
    });
  });

  it("accepts null recovery and rejects recovery fields outside the public projection", async () => {
    const runtime = new TauriCompanionRuntime();
    invokeMock.mockResolvedValueOnce(null);
    await expect(runtime.recoverNyxIdChat()).resolves.toBeNull();

    invokeMock.mockResolvedValueOnce({
      requestId,
      history: {
        conversation: { id: conversationId, activeTurn: true },
        messages: [],
      },
      ownerUserId: "must-not-cross-ipc",
    });
    await expect(runtime.recoverNyxIdChat()).rejects.toBeInstanceOf(ZodError);
  });

  it.each(["rejected", "admission_unknown"] as const)(
    "preserves a validated %s chat rejection",
    async (kind) => {
      invokeMock.mockRejectedValueOnce({
        kind,
        message: `public ${kind}`,
      });
      const runtime = new TauriCompanionRuntime();

      await expect(
        runtime.sendNyxIdChat({ requestId, text: "帮我处理" }),
      ).rejects.toMatchObject({
        name: "NyxIdChatSendError",
        kind,
        message: `public ${kind}`,
      });
    },
  );

  it("sanitizes malformed bridge rejections as admission unknown", async () => {
    invokeMock.mockRejectedValueOnce({
      kind: "rejected",
      message: "raw provider failure",
      accessToken: "must-not-cross-ipc",
    });
    const runtime = new TauriCompanionRuntime();

    let rejection: unknown;
    try {
      await runtime.sendNyxIdChat({ requestId, text: "帮我处理" });
    } catch (error) {
      rejection = error;
    }

    expect(rejection).toBeInstanceOf(NyxIdChatSendError);
    expect(rejection).toMatchObject({
      kind: "admission_unknown",
      message: NYXID_CHAT_ADMISSION_UNKNOWN_MESSAGE,
    });
    expect(String(rejection)).not.toContain("must-not-cross-ipc");
    expect(String(rejection)).not.toContain("raw provider failure");
  });

  it("forwards only validated chat events", async () => {
    const listener = vi.fn();
    const runtime = new TauriCompanionRuntime();
    await runtime.onNyxIdChatEvent(listener);

    expect(listenMock).toHaveBeenCalledWith(
      "companion://nyxid-chat",
      expect.any(Function),
    );
    nyxidEventHandler?.({
      payload: {
        kind: "delta",
        requestId,
        text: "公开回复",
      },
    });
    expect(listener).toHaveBeenCalledWith({
      kind: "delta",
      requestId,
      text: "公开回复",
    });

    nyxidEventHandler?.({
      payload: {
        kind: "delta",
        requestId,
        text: "公开回复",
        accessToken: "must-not-cross-ipc",
      },
    });
    expect(listener).toHaveBeenCalledOnce();
  });
});
