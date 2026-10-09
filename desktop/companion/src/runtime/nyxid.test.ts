import { describe, expect, it } from "vitest";

import { nyxIdViewSchema } from "./nyxid";

describe("NyxID frontend boundary", () => {
  it("accepts the public NyxID device approval projection", () => {
    expect(
      nyxIdViewSchema.parse({
        state: "authorizing",
        userCode: "ABCD-1234",
        verificationUrl:
          "https://nyx.chrono-ai.fun/login/device?user_code=ABCD-1234",
        expiresAt: "2026-10-09T10:10:00Z",
      }),
    ).toMatchObject({ state: "authorizing", userCode: "ABCD-1234" });
  });

  it("accepts only the sanitized connected projection", () => {
    const view = nyxIdViewSchema.parse({
      state: "connected",
      user: {
        id: "user-1",
        email: "ari@example.com",
        displayName: "Ari",
        avatarUrl: null,
      },
      capabilities: {
        enabledCount: 1,
        disabledCount: 0,
        attentionCount: 0,
        checkedAt: "2026-10-09T10:00:00Z",
        services: [
          {
            id: "service-1",
            slug: "api-github",
            label: "GitHub",
            state: "enabled",
          },
        ],
      },
    });

    expect(view.state).toBe("connected");
    expect(JSON.stringify(view)).not.toMatch(
      /access[_-]?token|refresh[_-]?token|device[_-]?code|bearer/i,
    );
  });

  it.each(["accessToken", "refreshToken", "deviceCode", "credential"])(
    "rejects an unexpected %s field from native payloads",
    (secretField) => {
      expect(() =>
        nyxIdViewSchema.parse({
          state: "authorizing",
          userCode: "ABCD-1234",
          verificationUrl:
            "https://nyx.chrono-ai.fun/login/device?user_code=ABCD-1234",
          expiresAt: "2026-10-09T10:10:00Z",
          [secretField]: "must-not-cross-ipc",
        }),
      ).toThrow();
    },
  );

  it.each([
    "javascript:alert(1)",
    "file:///tmp/device-login",
    "data:text/html,<script>alert(1)</script>",
    "https://attacker.example/login/device?user_code=ABCD-1234",
    "https://nyx.chrono-ai.fun/login/device?user_code=WRONG-CODE",
    "https://nyx.chrono-ai.fun/login/device?user_code=ABCD-1234&device_code=nyx_adc_secret",
    "https://nyx.chrono-ai.fun/login/device?user_code=ABCD-1234&access_token=secret",
    "https://nyx.chrono-ai.fun/login/device?user_code=ABCD-1234&refresh_token=secret",
    "https://nyx.chrono-ai.fun/login/device?user_code=ABCD-1234&credential=secret",
    "https://nyx.chrono-ai.fun/login/device?user_code=ABCD-1234#device_code=nyx_adc_secret",
  ])("rejects an unsafe device approval URL: %s", (verificationUrl) => {
    expect(() =>
      nyxIdViewSchema.parse({
        state: "authorizing",
        userCode: "ABCD-1234",
        verificationUrl,
        expiresAt: "2026-10-09T10:10:00Z",
      }),
    ).toThrow();
  });

  it("rejects invalid native timestamps", () => {
    expect(() =>
      nyxIdViewSchema.parse({
        state: "connected",
        user: { id: "user-1", email: "ari@example.com" },
        capabilities: {
          enabledCount: 0,
          disabledCount: 0,
          attentionCount: 0,
          checkedAt: "not-a-date",
          services: [],
        },
      }),
    ).toThrow();
  });

  it.each([
    ["connect", true],
    ["cancel", true],
    ["refresh", true],
    ["logout", true],
    [null, false],
  ] as const)(
    "accepts a consistent public retry action: %s",
    (retryAction, retryable) => {
      expect(
        nyxIdViewSchema.parse({
          state: "error",
          error: {
            code: "example_error",
            message: "请重试",
            retryable,
            retryAction,
          },
        }),
      ).toMatchObject({ state: "error", error: { retryAction } });
    },
  );

  it("rejects inconsistent retry metadata", () => {
    expect(() =>
      nyxIdViewSchema.parse({
        state: "error",
        error: {
          code: "example_error",
          message: "不能重试",
          retryable: false,
          retryAction: "connect",
        },
      }),
    ).toThrow();
  });
});
