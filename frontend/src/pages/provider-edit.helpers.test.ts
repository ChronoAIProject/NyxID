import { describe, expect, it } from "vitest";
import {
  PROVIDER_TYPE_LABELS,
  splitScopes,
  stripEmptyStrings,
} from "./provider-edit.helpers";

describe("stripEmptyStrings", () => {
  it("drops keys whose value is an empty string", () => {
    const result = stripEmptyStrings({
      name: "Provider",
      description: "",
      slug: "provider",
    });

    expect(result).toEqual({ name: "Provider", slug: "provider" });
    expect(result).not.toHaveProperty("description");
  });

  it("drops keys whose value is undefined", () => {
    const result = stripEmptyStrings({
      name: "Provider",
      credential_mode: undefined,
      supports_pkce: undefined,
    });

    expect(result).toEqual({ name: "Provider" });
  });

  it("keeps falsy-but-meaningful values like false and 0", () => {
    const result = stripEmptyStrings({
      supports_pkce: false,
      retries: 0,
      name: "Provider",
    });

    expect(result).toEqual({
      supports_pkce: false,
      retries: 0,
      name: "Provider",
    });
  });

  it("keeps array values such as parsed scopes", () => {
    const result = stripEmptyStrings({
      default_scopes: ["profile", "email"],
      token_url: "",
    });

    expect(result).toEqual({ default_scopes: ["profile", "email"] });
  });

  it("returns an empty object when every value is empty or undefined", () => {
    expect(stripEmptyStrings({ a: "", b: undefined })).toEqual({});
  });
});

describe("splitScopes", () => {
  it("splits a comma-separated string and trims each scope", () => {
    expect(splitScopes("profile, email , user:read")).toEqual([
      "profile",
      "email",
      "user:read",
    ]);
  });

  it("filters out blank segments left by trailing or doubled commas", () => {
    expect(splitScopes("profile,, ,email,")).toEqual(["profile", "email"]);
  });

  it("returns undefined for an undefined input", () => {
    expect(splitScopes(undefined)).toBeUndefined();
  });

  it("returns undefined for a whitespace-only string", () => {
    expect(splitScopes("   ")).toBeUndefined();
  });
});

describe("PROVIDER_TYPE_LABELS", () => {
  it("maps known provider types to human-readable labels", () => {
    expect(PROVIDER_TYPE_LABELS.oauth2).toBe("OAuth 2.0");
    expect(PROVIDER_TYPE_LABELS.api_key).toBe("API Key");
    expect(PROVIDER_TYPE_LABELS.device_code).toBe("Device Code");
    expect(PROVIDER_TYPE_LABELS.telegram_widget).toBe("Telegram Widget");
  });

  it("has no label for an unknown provider type", () => {
    expect(PROVIDER_TYPE_LABELS.unknown_type).toBeUndefined();
  });
});
