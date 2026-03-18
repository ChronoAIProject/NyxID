import { describe, expect, it } from "vitest";
import type { DownstreamService } from "@/types/api";
import {
  buildNodeCredentialCommand,
  getNodeCredentialPromptHint,
} from "./node-credentials";

function makeService(
  overrides: Partial<DownstreamService> = {},
): DownstreamService {
  return {
    id: "svc-1",
    name: "Example Service",
    slug: "example",
    description: "Example",
    base_url: "https://api.example.com",
    service_type: "http",
    auth_method: "header",
    auth_type: "api_key",
    auth_key_name: "X-API-Key",
    is_active: true,
    oauth_client_id: null,
    openapi_spec_url: null,
    api_spec_url: null,
    asyncapi_spec_url: null,
    streaming_supported: false,
    ssh_config: null,
    service_category: "connection",
    requires_user_credential: true,
    created_by: "user-1",
    created_at: "2026-03-10T00:00:00Z",
    updated_at: "2026-03-10T00:00:00Z",
    ...overrides,
  };
}

describe("buildNodeCredentialCommand", () => {
  it("uses secure bearer formatting for bearer services", () => {
    const service = makeService({
      auth_method: "bearer",
      auth_type: "bearer",
      auth_key_name: "Authorization",
    });

    expect(buildNodeCredentialCommand("openai", service)).toBe(
      "nyxid-node credentials add --service openai --header Authorization --secret-format bearer",
    );
    expect(getNodeCredentialPromptHint(service)).toContain("raw token");
  });

  it("uses secure basic formatting for basic services", () => {
    const service = makeService({
      auth_method: "basic",
      auth_type: "basic",
      auth_key_name: "Authorization",
    });

    expect(buildNodeCredentialCommand("github", service)).toBe(
      "nyxid-node credentials add --service github --header Authorization --secret-format basic",
    );
    expect(getNodeCredentialPromptHint(service)).toContain("username:password");
  });

  it("keeps query credentials as raw prompts", () => {
    const service = makeService({
      auth_method: "query",
      auth_type: "query",
      auth_key_name: "api_key",
    });

    expect(buildNodeCredentialCommand("stripe", service)).toBe(
      "nyxid-node credentials add --service stripe --query-param api_key",
    );
    expect(getNodeCredentialPromptHint(service)).toBeNull();
  });

  it("falls back to a raw header command when service metadata is unavailable", () => {
    expect(buildNodeCredentialCommand("fallback", undefined)).toBe(
      "nyxid-node credentials add --service fallback --header Authorization",
    );
  });
});
