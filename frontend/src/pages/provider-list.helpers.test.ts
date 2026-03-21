import { describe, expect, it } from "vitest";
import type { CreateProviderFormData } from "@/schemas/providers";
import {
  buildCreateProviderPayload,
  getProviderTypeFieldResets,
} from "./provider-list.helpers";

function makeFormData(
  overrides: Partial<CreateProviderFormData>,
): CreateProviderFormData {
  return {
    name: "Provider",
    slug: "provider",
    description: "",
    provider_type: "oauth2",
    credential_mode: "admin",
    authorization_url: "https://example.com/oauth/authorize",
    token_url: "https://example.com/oauth/token",
    revocation_url: "",
    default_scopes: "profile, email",
    client_id: "client-id",
    client_secret: "client-secret",
    supports_pkce: true,
    device_code_url: "",
    device_token_url: "",
    device_verification_url: "",
    hosted_callback_url: "",
    api_key_instructions: "",
    api_key_url: "",
    icon_url: "",
    documentation_url: "",
    token_endpoint_auth_method: "client_secret_post",
    extra_auth_params: undefined,
    device_code_format: "rfc8628",
    client_id_param_name: "",
    ...overrides,
  };
}

describe("buildCreateProviderPayload", () => {
  it("strips stale telegram bot usernames from non-telegram providers", () => {
    const payload = buildCreateProviderPayload(
      makeFormData({
        provider_type: "oauth2",
        client_id_param_name: "NyxIdBot",
      }),
    );

    expect(payload).not.toHaveProperty("client_id_param_name");
    expect(payload).toMatchObject({
      provider_type: "oauth2",
      client_id: "client-id",
      client_secret: "client-secret",
      default_scopes: ["profile", "email"],
    });
  });

  it("forces telegram widget payloads back to admin credential mode", () => {
    const payload = buildCreateProviderPayload(
      makeFormData({
        provider_type: "telegram_widget",
        credential_mode: "user",
        client_id_param_name: "NyxIdBot",
      }),
    );

    expect(payload).toMatchObject({
      provider_type: "telegram_widget",
      credential_mode: "admin",
      client_id_param_name: "NyxIdBot",
    });
    expect(payload).not.toHaveProperty("supports_pkce");
  });

  it("normalizes hidden credential_mode state for API key providers", () => {
    const payload = buildCreateProviderPayload(
      makeFormData({
        provider_type: "api_key",
        credential_mode: "both",
      }),
    );

    expect(payload).toMatchObject({
      provider_type: "api_key",
      credential_mode: "admin",
    });
  });
});

describe("getProviderTypeFieldResets", () => {
  it("forces admin mode when switching into telegram widget", () => {
    expect(getProviderTypeFieldResets("oauth2", "telegram_widget")).toEqual({
      credential_mode: "admin",
    });
  });

  it("clears telegram-only fields when switching away from telegram widget", () => {
    expect(getProviderTypeFieldResets("telegram_widget", "oauth2")).toEqual({
      client_id_param_name: "",
    });
  });
});
