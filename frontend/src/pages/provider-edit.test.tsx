import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { api } from "@/lib/api-client";
import type { ProviderConfig } from "@/types/api";
import { ProviderEditPage } from "./provider-edit";

vi.mock("@tanstack/react-router", () => ({
  useNavigate: () => vi.fn(),
  useParams: () => ({ providerId: "twitter-provider" }),
}));

vi.mock("sonner", () => ({ toast: { success: vi.fn(), error: vi.fn() } }));

const provider: ProviderConfig = {
  id: "twitter-provider",
  slug: "twitter",
  name: "Twitter / X",
  description: null,
  provider_type: "oauth2",
  has_oauth_config: true,
  credential_mode: "both",
  default_scopes: ["tweet.read", "tweet.write", "users.read"],
  supports_pkce: true,
  device_code_url: null,
  device_token_url: null,
  device_verification_url: null,
  hosted_callback_url: null,
  api_key_instructions: null,
  api_key_url: null,
  token_endpoint_auth_method: "client_secret_basic",
  extra_auth_params: null,
  device_code_format: "rfc8628",
  client_id_param_name: null,
  requires_gateway_url: false,
  icon_url: null,
  documentation_url: null,
  is_active: true,
  created_at: "2026-09-01T00:00:00Z",
  updated_at: "2026-09-01T00:00:00Z",
};

function renderEditor(credentialMode: ProviderConfig["credential_mode"] = "both") {
  vi.spyOn(api, "get").mockResolvedValue({ ...provider, credential_mode: credentialMode });
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  return render(
    <QueryClientProvider client={client}>
      <ProviderEditPage />
    </QueryClientProvider>,
  );
}

describe("ProviderEditPage", () => {
  beforeEach(() => vi.restoreAllMocks());

  it("submits OAuth app credentials without retyping the configured URLs", async () => {
    const put = vi.spyOn(api, "put").mockResolvedValue(provider);
    const user = userEvent.setup();
    renderEditor();

    const clientId = await screen.findByLabelText("Client ID");
    const save = screen.getByRole("button", { name: "Save Changes" });
    expect(save).toBeDisabled();
    await user.type(clientId, "test-client-id");
    await user.type(screen.getByLabelText("Client Secret"), "test-client-secret");
    expect(save).toBeEnabled();
    await user.click(save);

    await waitFor(() => expect(put).toHaveBeenCalledOnce());
    expect(put).toHaveBeenCalledWith("/providers/twitter-provider", expect.objectContaining({
      client_id: "test-client-id",
      client_secret: "test-client-secret",
      credential_mode: "both",
    }));
    const body = put.mock.calls[0]?.[1] as Record<string, unknown>;
    for (const field of ["authorization_url", "token_url", "revocation_url"]) {
      expect(body).not.toHaveProperty(field);
    }
  });

  it.each([
    ["user", "User Provided"],
    ["both", "Admin or User"],
  ] as const)("shows the fetched %s credential mode in the trigger", async (mode, label) => {
    renderEditor(mode);
    const trigger = await screen.findByRole("combobox", { name: "Credential Mode" });
    await waitFor(() => expect(trigger).toHaveTextContent(label));
  });
});
