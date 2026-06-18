import { describe, expect, it } from "vitest";
import {
  approveBodySchema,
  errorEnvelopeSchema,
  formatAuthDeviceUserCodeInput,
  friendlyAuthDeviceErrorMessage,
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
