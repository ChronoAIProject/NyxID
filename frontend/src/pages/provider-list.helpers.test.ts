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

  it("excludes stale OAuth fields from api_key payload", () => {
    // Simulates: user fills OAuth2 fields, then switches to api_key
    const payload = buildCreateProviderPayload(
      makeFormData({
        provider_type: "api_key",
        authorization_url: "https://example.com/oauth/authorize",
        token_url: "https://example.com/oauth/token",
        client_id: "leftover-client-id",
        client_secret: "leftover-client-secret",
        supports_pkce: true,
        api_key_instructions: "Get your key at ...",
        api_key_url: "https://example.com/keys",
      }),
    );

    expect(payload).not.toHaveProperty("authorization_url");
    expect(payload).not.toHaveProperty("token_url");
    expect(payload).not.toHaveProperty("client_id");
    expect(payload).not.toHaveProperty("client_secret");
    expect(payload).not.toHaveProperty("supports_pkce");
    expect(payload).toMatchObject({
      provider_type: "api_key",
      api_key_instructions: "Get your key at ...",
      api_key_url: "https://example.com/keys",
    });
  });

  it("excludes stale device_code fields from oauth2 payload", () => {
    const payload = buildCreateProviderPayload(
      makeFormData({
        provider_type: "oauth2",
        device_code_url: "https://example.com/device",
        device_token_url: "https://example.com/device/token",
      }),
    );

    expect(payload).not.toHaveProperty("device_code_url");
    expect(payload).not.toHaveProperty("device_token_url");
  });

  it("excludes stale OAuth fields from telegram_widget payload", () => {
    const payload = buildCreateProviderPayload(
      makeFormData({
        provider_type: "telegram_widget",
        authorization_url: "https://example.com/oauth/authorize",
        token_url: "https://example.com/oauth/token",
        client_id: "leftover-client-id",
        client_secret: "bot-token-123",
        client_id_param_name: "NyxIdBot",
      }),
    );

    expect(payload).not.toHaveProperty("authorization_url");
    expect(payload).not.toHaveProperty("token_url");
    expect(payload).not.toHaveProperty("client_id");
    expect(payload).toMatchObject({
      provider_type: "telegram_widget",
      client_secret: "bot-token-123",
      client_id_param_name: "NyxIdBot",
    });
  });
});

describe("getProviderTypeFieldResets", () => {
  it("forces admin mode when switching into telegram widget", () => {
    const resets = getProviderTypeFieldResets("oauth2", "telegram_widget");
    expect(resets.credential_mode).toBe("admin");
  });

  it("clears OAuth-exclusive fields when switching from oauth2 to telegram_widget", () => {
    const resets = getProviderTypeFieldResets("oauth2", "telegram_widget");
    expect(resets).toMatchObject({
      authorization_url: "",
      token_url: "",
      revocation_url: "",
      default_scopes: "",
      client_id: "",
      supports_pkce: false,
      credential_mode: "admin",
    });
  });

  it("returns empty resets when type does not change", () => {
    expect(getProviderTypeFieldResets("oauth2", "oauth2")).toEqual({});
  });

  it("clears OAuth-exclusive fields when switching from oauth2 to api_key", () => {
    const resets = getProviderTypeFieldResets("oauth2", "api_key");
    expect(resets).toMatchObject({
      authorization_url: "",
      token_url: "",
      revocation_url: "",
      default_scopes: "",
      client_id: "",
      client_secret: "",
      supports_pkce: false,
    });
  });

  it("clears device-code-exclusive fields when switching from device_code to oauth2", () => {
    const resets = getProviderTypeFieldResets("device_code", "oauth2");
    expect(resets).toMatchObject({
      device_code_url: "",
      device_token_url: "",
      device_verification_url: "",
    });
    // Fields shared between device_code and oauth2 should NOT be reset
    expect(resets).not.toHaveProperty("authorization_url");
    expect(resets).not.toHaveProperty("token_url");
    expect(resets).not.toHaveProperty("client_id");
    expect(resets).not.toHaveProperty("credential_mode");
  });

  it("clears api_key-exclusive fields when switching from api_key to oauth2", () => {
    const resets = getProviderTypeFieldResets("api_key", "oauth2");
    expect(resets).toMatchObject({
      api_key_instructions: "",
      api_key_url: "",
    });
  });

  it("regression: telegram_widget -> oauth2 clears bot username from form state", () => {
    // Review scenario: start with telegram_widget, enter NyxIdBot,
    // switch to oauth2. client_id_param_name must be cleared because
    // the create form does not render it for oauth2 (it would silently
    // become a custom OAuth param name on the backend).
    const resets = getProviderTypeFieldResets("telegram_widget", "oauth2");
    expect(resets.client_id_param_name).toBe("");
  });

  it("regression: telegram_widget -> oauth2 -> submit does not leak bot username", () => {
    // End-to-end: after applying resets and submitting, the payload
    // must not contain client_id_param_name for oauth2.
    const payload = buildCreateProviderPayload(
      makeFormData({
        provider_type: "oauth2",
        client_id_param_name: "NyxIdBot",
      }),
    );
    expect(payload).not.toHaveProperty("client_id_param_name");
  });
});
