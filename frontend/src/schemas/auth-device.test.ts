import { describe, expect, it } from "vitest";
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
  it("normalizes a missing client_ip from an older backend to null", () => {
    expect(
      previewResponseSchema.parse({
        client_label: "workstation",
        client_user_agent: "nyxid-cli",
        initiated_at: "2026-08-20T10:00:00Z",
        expires_at: "2026-08-20T10:10:00Z",
        status: "pending",
      }).client_ip,
    ).toBeNull();
  });
});

describe("browser device-code schemas", () => {
  it("accepts a request response and normalizes the request body", () => {
    expect(
      requestBodySchema.parse({
        client_label: "NyxID web (MacIntel)",
        client_user_agent: "Mozilla/5.0",
      }),
    ).toEqual({
      client_label: "NyxID web (MacIntel)",
      client_user_agent: "Mozilla/5.0",
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
