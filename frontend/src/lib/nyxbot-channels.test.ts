import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ApiError } from "@/lib/api-client";
import { AevatarAuthError } from "./nyxbot-aevatar-auth";
import {
  AEVATAR_CHANNELS_PATH,
  AEVATAR_WEBHOOK_BASE_URL,
  getNyxbotRegistrationStatus,
  registerNyxbotTelegram,
} from "./nyxbot-channels";

const { request, telegram, authorize, clearAuth } = vi.hoisted(() => ({
  request: vi.fn(),
  telegram: vi.fn(),
  authorize: vi.fn(),
  clearAuth: vi.fn(),
}));
vi.mock("./nyxbot-aevatar-auth", async (importOriginal) => ({
  ...(await importOriginal<typeof import("./nyxbot-aevatar-auth")>()),
  getAevatarAuthorization: authorize,
  clearAevatarAuthorization: clearAuth,
}));
vi.mock("@/lib/api-client", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/lib/api-client")>()),
  apiClient: request,
}));

const token = `123456:${"aB_9-".repeat(7)}`;
const identity = {
  ok: true,
  result: {
    id: 123456,
    is_bot: true,
    first_name: " My Shop Bot ",
    username: "my_shop_bot",
  },
};
const receipt = {
  status: "accepted",
  platform: "telegram",
  registration_id: "aevatar-id",
  nyx_channel_bot_id: "nyx-bot-id",
};
const json = (body: unknown, status = 200) =>
  new Response(JSON.stringify(body), { status });

beforeEach(() => {
  vi.resetAllMocks();
  vi.stubGlobal("fetch", telegram);
  telegram.mockResolvedValue(json(identity));
  request.mockResolvedValue(receipt);
  authorize.mockResolvedValue("Bearer test-oauth-access");
});
afterEach(() => vi.unstubAllGlobals());

describe("Nyxbot Telegram registration", () => {
  it("gets the bot name before sending the exact registration fields to Aevatar", async () => {
    await expect(registerNyxbotTelegram(`  ${token}  `)).resolves.toEqual({
      ...receipt,
      telegram_url: "https://t.me/my_shop_bot",
    });
    expect(telegram).toHaveBeenCalledWith(
      `https://api.telegram.org/bot${token}/getMe`,
      expect.objectContaining({
        method: "POST",
        credentials: "omit",
        cache: "no-store",
        referrerPolicy: "no-referrer",
        redirect: "error",
        signal: expect.any(AbortSignal),
      }),
    );
    expect(telegram.mock.invocationCallOrder[0]).toBeLessThan(
      request.mock.invocationCallOrder[0]!,
    );
    expect(request).toHaveBeenCalledExactlyOnceWith(AEVATAR_CHANNELS_PATH, {
      method: "POST",
      headers: { Authorization: "Bearer test-oauth-access" },
      body: {
        platform: "telegram",
        bot_token: token,
        label: "My_Shop_Bot_nyxid_bot",
        webhook_base_url: AEVATAR_WEBHOOK_BASE_URL,
        service_ids: [],
      },
      preserveSessionOn401: true,
      signal: expect.any(AbortSignal),
    });
  });
  it("passes the requested active service IDs and removes duplicates", async () => {
    await registerNyxbotTelegram(token, [
      "workspace-service",
      "ornn-service",
      "workspace-service",
    ]);
    expect(request).toHaveBeenCalledWith(
      AEVATAR_CHANNELS_PATH,
      expect.objectContaining({
        body: expect.objectContaining({
          service_ids: ["workspace-service", "ornn-service"],
        }),
      }),
    );
  });
  it("does not register while Telegram verification is still pending", async () => {
    let resolveIdentity: ((response: Response) => void) | undefined;
    telegram.mockImplementation(
      () =>
        new Promise((resolve) => {
          resolveIdentity = resolve;
        }),
    );
    const pending = registerNyxbotTelegram(token);
    expect(request).not.toHaveBeenCalled();
    resolveIdentity?.(json(identity));
    await pending;
    expect(request).toHaveBeenCalledTimes(1);
  });
  it.each([401, 404])(
    "stops before registration when Telegram rejects the token with %i",
    async (status) => {
      telegram.mockResolvedValue(
        json({ ok: false, description: `Rejected ${token}` }, status),
      );
      await expect(registerNyxbotTelegram(token)).rejects.toMatchObject({
        code: "tokenRejected",
        message: "tokenRejected",
      });
      expect(request).not.toHaveBeenCalled();
    },
  );
  it("handles Telegram's unsuccessful API envelope without registering", async () => {
    telegram.mockResolvedValue(json({ ok: false, error_code: 401 }));
    await expect(registerNyxbotTelegram(token)).rejects.toMatchObject({
      code: "tokenRejected",
    });
    expect(request).not.toHaveBeenCalled();
  });
  it.each([
    { ok: true, result: { ...identity.result, is_bot: false } },
    { ok: true, result: { ...identity.result, first_name: " " } },
    { ok: true, result: { ...identity.result, username: undefined } },
    { ok: true, result: { ...identity.result, username: "../another_bot" } },
    { ok: true },
  ])("requires an actual bot identity and name: %j", async (response) => {
    telegram.mockResolvedValue(json(response));
    await expect(registerNyxbotTelegram(token)).rejects.toMatchObject({
      code: "telegramUnavailable",
    });
    expect(request).not.toHaveBeenCalled();
  });
  it("retains non-Latin names when normalizing label spaces", async () => {
    telegram.mockResolvedValue(
      json({
        ok: true,
        result: { ...identity.result, first_name: "客服 助手" },
      }),
    );
    await registerNyxbotTelegram(token);
    expect(request).toHaveBeenCalledWith(
      AEVATAR_CHANNELS_PATH,
      expect.objectContaining({
        body: expect.objectContaining({ label: "客服_助手_nyxid_bot" }),
      }),
    );
  });
  it.each(["Test01", "Test01_nyxid_bot"])(
    "uses one suffix for %s",
    async (name) => {
      telegram.mockResolvedValue(
        json({ ...identity, result: { ...identity.result, first_name: name } }),
      );
      await registerNyxbotTelegram(token);
      expect(request).toHaveBeenCalledWith(
        AEVATAR_CHANNELS_PATH,
        expect.objectContaining({
          body: expect.objectContaining({ label: "Test01_nyxid_bot" }),
        }),
      );
    },
  );
  it("bounds Unicode labels to NyxID's UTF-8 byte limit including suffix", async () => {
    telegram.mockResolvedValue(
      json({
        ...identity,
        result: { ...identity.result, first_name: "助".repeat(128) },
      }),
    );
    await registerNyxbotTelegram(token);
    const label = request.mock.calls[0]![1].body.label as string;
    expect(new TextEncoder().encode(label).length).toBeLessThanOrEqual(200);
    expect(label).toMatch(/^助+_nyxid_bot$/);
  });
  it("requires existing Aevatar consent before registration", async () => {
    authorize.mockRejectedValue(new AevatarAuthError("channelConsentRequired"));
    await expect(registerNyxbotTelegram(token)).rejects.toMatchObject({
      code: "channelConsentRequired",
    });
    expect(request).not.toHaveBeenCalled();
  });
  it("drops credential-bearing network errors and does not register", async () => {
    telegram.mockRejectedValue(
      new Error(`Request failed: https://api.telegram.org/bot${token}/getMe`),
    );
    await expect(registerNyxbotTelegram(token)).rejects.toMatchObject({
      code: "telegramUnavailable",
      message: "telegramUnavailable",
    });
    expect(request).not.toHaveBeenCalled();
  });
  it.each([401, 403])(
    "reports Aevatar auth failure %i separately without replaying registration",
    async (status) => {
      request.mockRejectedValue(
        new ApiError(status, {
          error: "unauthorized",
          error_code: 1,
          message: token,
        }),
      );
      await expect(registerNyxbotTelegram(token)).rejects.toMatchObject({
        code: "channelAuthRequired",
        message: "channelAuthRequired",
      });
      expect(request).toHaveBeenCalledTimes(1);
      expect(clearAuth).toHaveBeenCalledTimes(status === 401 ? 1 : 0);
    },
  );
  it.each([
    { ...receipt, registration_id: "" },
    { ...receipt, nyx_channel_bot_id: "" },
    { ...receipt, status: "error" },
    { id: "legacy-nyx-bot", status: "active" },
  ])(
    "does not treat an invalid receipt as a registered channel: %j",
    async (response) => {
      request.mockResolvedValue(response);
      await expect(registerNyxbotTelegram(token)).rejects.toMatchObject({
        code: "channelRegistrationFailed",
      });
      expect(request).toHaveBeenCalledTimes(1);
    },
  );
  it("queries status using the Aevatar registration ID independently of the NyxID bot ID", async () => {
    const status = { ...receipt, status: "active" };
    request.mockResolvedValue(status);
    await expect(getNyxbotRegistrationStatus("aevatar-id")).resolves.toEqual({
      registration_id: "aevatar-id",
      nyx_channel_bot_id: "nyx-bot-id",
      status: "active",
    });
    expect(request).toHaveBeenCalledWith(
      `${AEVATAR_CHANNELS_PATH}/aevatar-id/status`,
      {
        preserveSessionOn401: true,
        headers: { Authorization: "Bearer test-oauth-access" },
      },
    );
  });
});
