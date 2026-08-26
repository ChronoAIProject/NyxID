import { describe, expect, it } from "vitest";
import { ApiError } from "@/lib/api-client";
import {
  approveBodySchema,
  denyBodySchema,
  errorEnvelopeSchema,
  formatAuthDeviceUserCodeInput,
  friendlyAuthDeviceErrorMessage,
  friendlyAuthDeviceStatusMessage,
  pollBodySchema,
  pollWebResponseSchema,
  previewResponseSchema,
  requestBodySchema,
  requestResponseSchema,
  userCodeSchema,
} from "./auth-device";

describe("userCodeSchema", () => {
  it("normalizes case and separators", () => {
    expect(userCodeSchema.parse("abcd-efgh")).toBe("ABCDEFGH");
    expect(userCodeSchema.parse("abcd efgh")).toBe("ABCDEFGH");
    expect(userCodeSchema.parse("abCD\tefGH")).toBe("ABCDEFGH");
  });

  it("rejects 7-character and 9-character inputs", () => {
    expect(userCodeSchema.safeParse("ABCDEFG").success).toBe(false);
    expect(userCodeSchema.safeParse("ABCDEFGHI").success).toBe(false);
  });

  it("rejects ambiguous I, L, O, and U inputs", () => {
    for (const char of ["I", "L", "O", "U"]) {
      expect(userCodeSchema.safeParse(`ABCD-EFG${char}`).success).toBe(false);
    }
  });
});

describe("approveBodySchema", () => {
  it("normalizes the request payload", () => {
    expect(approveBodySchema.parse({ user_code: "abcd-efgh" })).toEqual({
      user_code: "ABCDEFGH",
    });
  });
});

describe("denyBodySchema", () => {
  it("normalizes the request payload", () => {
    expect(denyBodySchema.parse({ user_code: "abcd-efgh" })).toEqual({
      user_code: "ABCDEFGH",
    });
  });
});

describe("previewResponseSchema", () => {
  it("normalizes additive fields missing from an older backend", () => {
    expect(
      previewResponseSchema.parse({
        client_label: "workstation",
        client_user_agent: "nyxid-cli",
        initiated_at: "2026-08-20T10:00:00Z",
        expires_at: "2026-08-20T10:10:00Z",
        status: "pending",
      }),
    ).toMatchObject({
      client_ip: null,
      client_ip_attribution: "unavailable",
      client_country: null,
      client_kind: "unknown",
      client_app: null,
      client_platform: null,
      same_ip_as_viewer: null,
      seconds_remaining: null,
      initiating_origin: null,
      initiating_origin_status: "absent",
      network_relation: null,
      client_timezone: null,
      client_ip_timezone: null,
      client_timezone_matches_ip: null,
    });
  });

  it("accepts the verbose requester attribution fields", () => {
    expect(
      previewResponseSchema.parse({
        client_label: "workstation",
        client_user_agent: "nyxid-cli/1.4.2 (macos; aarch64)",
        client_ip: "203.0.113.10",
        client_ip_attribution: "verified",
        client_country: "SG",
        client_kind: "cli",
        client_app: "NyxID CLI 1.4.2",
        client_platform: "macOS (aarch64)",
        same_ip_as_viewer: false,
        seconds_remaining: 583,
        initiating_origin: "https://nyxid.dev",
        initiating_origin_status: "matched",
        network_relation: "same_network",
        client_city: "Singapore",
        client_region: "Singapore",
        client_continent: "AS",
        client_ip_timezone: "Asia/Singapore",
        client_timezone: "Europe/Moscow",
        client_timezone_matches_ip: false,
        client_locale: "en-SG",
        client_form_factor: "desktop",
        client_screen_width: 1512,
        client_screen_height: 982,
        client_device_pixel_ratio: 2,
        client_hardware_concurrency: 12,
        client_device_memory: 16,
        initiated_at: "2026-08-20T10:00:00Z",
        expires_at: "2026-08-20T10:10:00Z",
        status: "pending",
      }),
    ).toMatchObject({
      client_country: "SG",
      client_ip_attribution: "verified",
      client_kind: "cli",
      client_app: "NyxID CLI 1.4.2",
      client_platform: "macOS (aarch64)",
      same_ip_as_viewer: false,
      seconds_remaining: 583,
      initiating_origin_status: "matched",
      network_relation: "same_network",
      client_city: "Singapore",
      client_timezone_matches_ip: false,
      client_form_factor: "desktop",
    });
  });

  it("bounds and strips control characters from requester display fields", () => {
    const parsed = previewResponseSchema.parse({
      client_label: `host\u0000${"x".repeat(100)}`,
      client_user_agent: `agent\n${"y".repeat(300)}`,
      initiated_at: "2026-08-20T10:00:00Z",
      expires_at: "2026-08-20T10:10:00Z",
      status: "pending",
    });

    expect(parsed.client_label).toHaveLength(64);
    expect(parsed.client_user_agent).toHaveLength(256);
    expect(parsed.client_label).not.toContain("\u0000");
    expect(parsed.client_user_agent).not.toContain("\n");
  });
});

describe("browser device-code schemas", () => {
  it("accepts a request response and normalizes the request body", () => {
    expect(
      requestBodySchema.parse({
        client_label: "Chrome 131 on macOS 15.2",
        client_user_agent: "Mozilla/5.0",
        client_app: "Chrome 131",
        client_platform: "macOS 15.2 (arm64)",
        client_form_factor: "desktop",
        client_timezone: "Asia/Singapore",
        client_locale: "en-SG",
        client_screen_width: 1512,
        client_screen_height: 982,
        client_device_pixel_ratio: 2,
        client_hardware_concurrency: 12,
        client_device_memory: 16,
      }),
    ).toEqual({
      client_label: "Chrome 131 on macOS 15.2",
      client_user_agent: "Mozilla/5.0",
      client_app: "Chrome 131",
      client_platform: "macOS 15.2 (arm64)",
      client_form_factor: "desktop",
      client_timezone: "Asia/Singapore",
      client_locale: "en-SG",
      client_screen_width: 1512,
      client_screen_height: 982,
      client_device_pixel_ratio: 2,
      client_hardware_concurrency: 12,
      client_device_memory: 16,
    });
    expect(
      requestResponseSchema.parse({
        device_code: "nyx_adc_test",
        user_code: "ABCD-EFGH",
        verification_uri: "https://id.example/login/device",
        verification_uri_complete:
          "https://id.example/login/device?user_code=ABCD-EFGH",
        expires_in: 600,
        interval: 5,
      }).interval,
    ).toBe(5);
  });

  it("validates the cookie-only web poll response", () => {
    expect(pollBodySchema.parse({ device_code: "nyx_adc_test" })).toEqual({
      device_code: "nyx_adc_test",
    });
    expect(pollWebResponseSchema.parse({ ok: true })).toEqual({ ok: true });
    expect(pollWebResponseSchema.safeParse({ access_token: "secret" }).success).toBe(false);
  });
});

describe("formatAuthDeviceUserCodeInput", () => {
  it("keeps an editable XXXX-XXXX shape while typing", () => {
    expect(formatAuthDeviceUserCodeInput("ab")).toBe("AB");
    expect(formatAuthDeviceUserCodeInput("abcde")).toBe("ABCD-E");
    expect(formatAuthDeviceUserCodeInput("abcd-efgh-zz")).toBe("ABCD-EFGH");
  });
});

describe("errorEnvelopeSchema", () => {
  it("accepts the documented error envelope shape", () => {
    expect(
      errorEnvelopeSchema.parse({
        error: "auth_device_authorization_pending",
        error_code: 11202,
        message: "Authorization pending.",
      }),
    ).toEqual({
      error: "auth_device_authorization_pending",
      error_code: 11202,
      message: "Authorization pending.",
    });
  });
});

describe("friendlyAuthDeviceErrorMessage", () => {
  it("maps auth-device error codes to friendly messages", () => {
    expect(
      friendlyAuthDeviceErrorMessage({
        errorCode: 11204,
      }),
    ).toBe("This login request was already denied.");

    expect(
      friendlyAuthDeviceErrorMessage({
        errorCode: 11200,
        errorResponse: {
          error: "auth_device_code_not_found",
          error_code: 11200,
          message: "Not found.",
        },
      }),
    ).toBe("That code is no longer valid. Run `nyxid login --device` again.");

    expect(
      friendlyAuthDeviceErrorMessage({
        errorResponse: {
          error: "auth_device_expired_token",
          error_code: 11201,
          message: "Expired.",
        },
      }),
    ).toBe("This code has expired.");

    expect(
      friendlyAuthDeviceErrorMessage({
        errorCode: 11205,
      }),
    ).toBe("This code was already used.");

    expect(
      friendlyAuthDeviceErrorMessage({
        errorCode: 11206,
      }),
    ).toBe("Too many attempts. Try again in a few minutes.");

    expect(
      friendlyAuthDeviceErrorMessage({
        errorCode: 11207,
      }),
    ).toBe("That code is no longer valid. Run `nyxid login --device` again.");
  });

  it("does not expose transport error details", () => {
    expect(
      friendlyAuthDeviceErrorMessage(
        new Error("Request failed with status 502"),
      ),
    ).toBe("Couldn't reach NyxID. Check your connection and try again.");
  });

  it("does not expose an unmapped API error message", () => {
    expect(
      friendlyAuthDeviceErrorMessage(
        new ApiError(502, {
          error: "upstream_failure",
          error_code: 19999,
          message: "Proxy returned an invalid upstream response",
        }),
      ),
    ).toBe("Couldn't reach NyxID. Check your connection and try again.");
  });
});

describe("friendlyAuthDeviceStatusMessage", () => {
  it.each([
    ["denied", "This login request was already denied."],
    ["expired", "This code has expired."],
    ["approved", "This code was already used."],
    ["delivered", "This code was already used."],
  ] as const)("maps %s previews to terminal copy", (status, message) => {
    expect(friendlyAuthDeviceStatusMessage(status)).toBe(message);
  });

  it("returns no message for a pending preview", () => {
    expect(friendlyAuthDeviceStatusMessage("pending")).toBeNull();
  });
});
