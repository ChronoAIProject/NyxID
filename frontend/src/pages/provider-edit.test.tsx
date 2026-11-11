vi.mock("@/components/providers/provider-services", () => ({
  ProviderServices: () => <div>Linked service configuration</div>,
}));
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
  authorization_url: "https://twitter.com/i/oauth2/authorize",
  token_url: "https://api.twitter.com/2/oauth2/token",
  has_client_id: true,
  has_client_secret: true,
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

function renderEditor(
  credentialMode: ProviderConfig["credential_mode"] = "both",
  authorizationNonceRequired?: boolean,
) {
  vi.spyOn(api, "get").mockResolvedValue({
    ...provider,
    credential_mode: credentialMode,
    authorization_nonce_required: authorizationNonceRequired,
  });
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

    const clientId = await screen.findByLabelText(/^Client ID/);
    const save = screen.getByRole("button", { name: "Save Changes" });
    expect(save).toBeDisabled();
    await user.type(clientId, "test-client-id");
    await user.type(
      screen.getByLabelText(/^Client Secret/),
      "test-client-secret",
    );
    expect(save).toBeEnabled();
    await user.click(save);
    const review = await screen.findByRole("dialog", {
      name: "Review changes",
    });
    expect(put).not.toHaveBeenCalled();
    expect(review).not.toHaveTextContent("test-client-id");
    expect(review).not.toHaveTextContent("test-client-secret");
    await user.click(screen.getByRole("button", { name: "Confirm changes" }));

    await waitFor(() => expect(put).toHaveBeenCalledOnce());
    expect(put).toHaveBeenCalledWith("/providers/twitter-provider", {
      client_id: "test-client-id",
      client_secret: "test-client-secret",
    });
    const body = put.mock.calls[0]?.[1] as Record<string, unknown>;
    for (const field of ["authorization_url", "token_url", "revocation_url"]) {
      expect(body).not.toHaveProperty(field);
    }
  });

  it("preserves enabled nonce when changing only the provider name", async () => {
    const put = vi.spyOn(api, "put").mockResolvedValue({ ...provider, authorization_nonce_required: true });
    const user = userEvent.setup();
    renderEditor("both", true);
    const name = await screen.findByLabelText("Name");
    await user.clear(name);
    await user.type(name, "Updated provider");
    await user.click(screen.getByRole("button", { name: "Save Changes" }));
    await user.click(await screen.findByRole("button", { name: "Confirm changes" }));
    await waitFor(() => expect(put).toHaveBeenCalledWith("/providers/twitter-provider", { name: "Updated provider" }));
    expect(screen.getByRole("switch", { name: "Authorization nonce required" })).toBeChecked();
  });

  it.each([undefined, false, true])(
    "edits the hosted authorization nonce from %s and submits only that setting",
    async (enabled) => {
      const put = vi.spyOn(api, "put").mockResolvedValue(provider);
      const user = userEvent.setup();
      renderEditor("both", enabled);
      const toggle = await screen.findByRole("switch", { name: "Authorization nonce required" });
      expect(toggle).toHaveAttribute("aria-checked", String(enabled ?? false));
      const save = screen.getByRole("button", { name: "Save Changes" });
      expect(save).toBeDisabled();
      await user.click(toggle);
      expect(save).toBeEnabled();
      await user.click(save);
      await user.click(await screen.findByRole("button", { name: "Confirm changes" }));
      await waitFor(() => expect(put).toHaveBeenCalledWith("/providers/twitter-provider", {
        authorization_nonce_required: !(enabled ?? false),
      }));
    },
  );

  it.each([
    ["user", "User Provided"],
    ["both", "Admin or User"],
  ] as const)(
    "shows the fetched %s credential mode in the trigger",
    async (mode, label) => {
      renderEditor(mode);
      const trigger = await screen.findByRole("combobox", {
        name: "Credential Mode",
      });
      await waitFor(() => expect(trigger).toHaveTextContent(label));
    },
  );
});
