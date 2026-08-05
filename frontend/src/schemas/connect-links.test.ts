import { describe, expect, it } from "vitest";
import {
  connectCredentialFormSchema,
  connectLinkPreviewSchema,
  connectLinkStatusResponseSchema,
  connectOAuthFormSchema,
  validateConnectCredentialForm,
  validateConnectOAuthForm,
} from "@/schemas/connect-links";

describe("connect link schemas", () => {
  it("accepts the non-sensitive preview contract", () => {
    const parsed = connectLinkPreviewSchema.parse({
      service_name: "GitHub",
      service_slug: "github",
      label: "Coding agent",
      requested_by: "codex",
      created_at: "2026-08-05T10:00:00Z",
      expires_at: "2026-08-05T10:15:00Z",
      status: "pending",
      connect_method: "oauth",
      auth_key_name: "Authorization",
      credential_mode: "admin",
      has_platform_oauth_credentials: true,
      requires_gateway_url: false,
      api_key_url: null,
      api_key_instructions: null,
      callback_url:
        "desktop-app://connect/return?status=cancelled&connect_link_id=65dd8fe8-9ee8-4c89-af1e-b283a17bcf37",
    });
    expect(parsed.service_slug).toBe("github");
    expect(parsed).not.toHaveProperty("token");
    expect(parsed.callback_url).toContain("status=cancelled");
  });

  it("validates BYO OAuth and gateway fields independently", () => {
    const empty = connectOAuthFormSchema.parse({
      endpoint_url: "",
      oauth_client_id: "",
      oauth_client_secret: "",
    });
    expect(validateConnectOAuthForm(empty, false, false)).toBeNull();
    expect(validateConnectOAuthForm(empty, true, false)).toMatch(/Endpoint URL/);
    expect(validateConnectOAuthForm(empty, false, true)).toMatch(
      /ID and secret are required/,
    );

    const complete = connectOAuthFormSchema.parse({
      endpoint_url: "https://gateway.example.test",
      oauth_client_id: "client-id",
      oauth_client_secret: "client-secret",
    });
    expect(validateConnectOAuthForm(complete, true, true)).toBeNull();
  });

  it("accepts app identity and provider-decline status metadata", () => {
    const parsed = connectLinkStatusResponseSchema.parse({
      id: "65dd8fe8-9ee8-4c89-af1e-b283a17bcf37",
      status: "pending",
      service_name: "GitHub",
      service_slug: "github",
      expires_at: "2026-08-05T10:15:00Z",
      requesting_app_id: "desktop-client",
      requesting_app_name: "Desktop App",
      last_error: "provider_access_denied",
      last_error_at: "2026-08-05T10:04:12Z",
    });
    expect(parsed.last_error).toBe("provider_access_denied");
    expect(parsed.requesting_app_name).toBe("Desktop App");
  });

  it("validates credentials and service-specific endpoint requirements", () => {
    const values = connectCredentialFormSchema.parse({
      credential: "secret",
      endpoint_url: "",
      oauth_client_id: "",
      oauth_client_secret: "",
    });
    expect(validateConnectCredentialForm(values, false)).toBeNull();
    expect(validateConnectCredentialForm(values, true)).toMatch(/Endpoint URL/);
  });

  it("rejects partial OAuth client credentials", () => {
    const values = connectCredentialFormSchema.parse({
      credential: "secret",
      endpoint_url: "https://gateway.example.test",
      oauth_client_id: "client-id",
      oauth_client_secret: "",
    });
    expect(validateConnectCredentialForm(values, true)).toMatch(/supplied together/);
  });
});
