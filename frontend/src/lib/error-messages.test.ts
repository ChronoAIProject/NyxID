import { afterEach, describe, expect, it } from "vitest";
import { appI18n } from "@/i18n/instance";
import { apiErrorBody } from "./api-error";
import { ApiError } from "./api-client";
import { translateErrorMessage } from "./error-messages";

afterEach(async () => {
  await appI18n.changeLanguage("en");
});

describe.each(["zh-CN", "zh-TW"])("error presentation in %s", (language) => {
  it("translates known failures while retaining raw API errors and codes", async () => {
    const response = {
      error: "token_expired",
      error_code: 2001,
      message: "Token expired",
    };
    const error = new ApiError(401, response);
    await appI18n.changeLanguage(language);

    expect(translateErrorMessage(error.message)).not.toBe("Token expired");
    expect(error.message).toBe("Token expired");
    expect(error.errorResponse).toBe(response);
    expect(error.errorCode).toBe(2001);
    expect(error.status).toBe(401);
    expect(error.name).toBe("ApiError");
  });

  it("keeps unrecognized provider details, resource names, and HTTP status intact", async () => {
    await appI18n.changeLanguage(language);
    const detail =
      "Provider custom error: Profile <test> https://example.test/a";
    expect(translateErrorMessage(detail)).toBe(detail);
    expect(translateErrorMessage("Profile")).toBe("Profile");
    expect(translateErrorMessage(`Bad request: ${detail}`)).toContain(detail);
    expect(translateErrorMessage("Request failed with status 503")).toContain(
      "503",
    );
    expect(
      translateErrorMessage("Authentication failed: Invalid credentials"),
    ).not.toMatch(/Authentication failed|Invalid credentials/);
  });

  it("distinguishes fixed billing and connect errors without relying on numeric codes", async () => {
    await appI18n.changeLanguage(language);
    const billing = translateErrorMessage("Insufficient billing credits");
    const connect = translateErrorMessage("Connect link not found");
    expect(billing).not.toBe("Insufficient billing credits");
    expect(connect).not.toBe("Connect link not found");
    expect(billing).not.toBe(connect);
    expect(translateErrorMessage("Failed to fetch")).toBe(
      translateErrorMessage(
        "Couldn't reach NyxID. Check your connection and try again.",
      ),
    );
  });

  it("translates voice and chat failures without changing diagnostic values", async () => {
    await appI18n.changeLanguage(language);
    const raw =
      "Voice startup failed (OpenAI 429: rate_limit · code 12500)\nTrace ID: trace_12345678";
    const displayed = translateErrorMessage(raw);
    expect(displayed).not.toContain("Voice startup failed");
    expect(displayed).toContain("OpenAI 429: rate_limit");
    expect(displayed).toContain("12500");
    expect(displayed).toContain("trace_12345678");
    const unknown = "Custom provider response (code 12345)";
    expect(translateErrorMessage(unknown)).toBe(unknown);
  });
});

it("honors an explicit language without mutating the shared preference", async () => {
  await appI18n.changeLanguage("en");
  expect(translateErrorMessage("Token expired", { lng: "zh-TW" })).not.toBe(
    "Token expired",
  );
  expect(appI18n.language).toBe("en");
});


it.each([0, 503, 429, 401, 400])("translates HTTP %s fallback copy in both Chinese catalogs without changing the error", async (status) => {
  const body = apiErrorBody(null, status);
  const error = new ApiError(status, body);
  for (const language of ["zh-CN", "zh-TW"]) {
    await appI18n.changeLanguage(language);
    const displayed = translateErrorMessage(error.message);
    expect(displayed).not.toBe(body.message);
    expect(displayed).toMatch(/[\u3400-\u9fff]/);
    expect(error.message).toBe(body.message);
    expect(error.status).toBe(status);
  }
});
