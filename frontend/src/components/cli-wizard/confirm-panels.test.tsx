import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { PropsWithChildren } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";

// Issue #787 — CLI-wizard confirm-panels coverage. Each exported panel
// owns one pairing "confirm" step: render the prefill summary, let the
// user tweak a field or two, then fire the mint/rotate API call and hand
// a typed success result back via `onSuccess`. These tests pin, per the
// #787 acceptance item ("confirm-panels submit happy path + validation-
// error path"), for each panel:
//   - the rendered summary / prefilled inputs (the values it confirms);
//   - the HAPPY path: the confirm action calls the right endpoint with
//     the right body and fires `onSuccess` with the typed shape;
//   - a validation / error path: submit is blocked when required input
//     is missing, OR a surfaced backend error appears inline.

const { mockGet, mockPost, mockUseOrgs, mockUseKeys, mockUseNodes } =
  vi.hoisted(() => ({
    mockGet: vi.fn(),
    mockPost: vi.fn(),
    mockUseOrgs: vi.fn(),
    mockUseKeys: vi.fn(),
    mockUseNodes: vi.fn(),
  }));

vi.mock("@/lib/api-client", () => ({
  api: {
    post: mockPost,
    get: mockGet,
  },
  ApiError: class ApiError extends Error {
    status: number;
    errorCode: number;
    constructor(
      status: number,
      response: { message: string; error_code: number },
    ) {
      super(response.message);
      this.status = status;
      this.errorCode = response.error_code;
    }
  },
}));

// The wizard's "reserve action" + "rewind on error" helpers do their own
// backend calls to the cli-pairings endpoint. They're orthogonal to the
// panel contracts under test: reserve resolves, rewind just runs the
// wrapped fn. Stub both. We assert reserve is called on the happy path so
// the "reserve THEN mint" ordering stays pinned.
const { mockReserve, mockRewind } = vi.hoisted(() => ({
  mockReserve: vi.fn(),
  mockRewind: vi.fn(),
}));
vi.mock("@/pages/cli-pair/reserve-action", () => ({
  reservePairingAction: mockReserve,
  withRewindOnError: mockRewind,
}));

// useOrgs drives the optional "Owner" picker; useKeys/useNodes feed the
// AccessScopeCard's service/node lists. None hit the network in tests.
vi.mock("@/hooks/use-orgs", () => ({ useOrgs: mockUseOrgs }));
vi.mock("@/hooks/use-keys", () => ({ useKeys: mockUseKeys }));
vi.mock("@/hooks/use-nodes", () => ({ useNodes: mockUseNodes }));

// MfaSetupConfirm renders the otpauth secret as a QR. The qrcode lib does
// canvas work happy-dom can't do; stub it to a data URL.
vi.mock("qrcode", () => ({
  default: { toDataURL: vi.fn().mockResolvedValue("data:image/png;base64,QR") },
}));

import {
  ApiKeyCreateConfirm,
  ApiKeyRotateConfirm,
  NodeRegisterConfirm,
  NodeRotateConfirm,
  ServiceAccountCreateConfirm,
  ServiceAccountRotateSecretConfirm,
  DeveloperAppCreateConfirm,
  DeveloperAppRotateSecretConfirm,
  MfaSetupConfirm,
} from "./confirm-panels";

function createWrapper() {
  const queryClient = new QueryClient({
    defaultOptions: {
      mutations: { retry: false },
      queries: { retry: false },
    },
  });
  return function Wrapper({ children }: PropsWithChildren) {
    return (
      <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
    );
  };
}

function bodyForCall(path: string): Record<string, unknown> {
  const call = mockPost.mock.calls.find(([p]) => p === path);
  if (!call) throw new Error(`missing POST ${path} call`);
  return (call[1] ?? {}) as Record<string, unknown>;
}

const pairingId = "pair-test-123";

beforeEach(() => {
  mockGet.mockReset();
  mockPost.mockReset();
  mockUseOrgs.mockReset();
  mockUseKeys.mockReset();
  mockUseNodes.mockReset();
  mockReserve.mockReset();
  mockRewind.mockReset();

  mockUseOrgs.mockReturnValue({ data: [] });
  // AccessScopeCard's lists default to "allow all", which hides the list,
  // but the hooks still run on render — give them inert empty results.
  mockUseKeys.mockReturnValue({ data: [], isLoading: false });
  mockUseNodes.mockReturnValue({ data: [], isLoading: false });

  mockReserve.mockResolvedValue(undefined);
  // withRewindOnError(id, run) just runs and returns the wrapped call.
  mockRewind.mockImplementation(
    async (_id: string, run: () => Promise<unknown>) => run(),
  );
});

// ── ApiKeyCreateConfirm ──────────────────────────────────────────────

describe("ApiKeyCreateConfirm", () => {
  it("prefills name/platform/scopes and POSTs /api-keys with the assembled body, then fires onSuccess", async () => {
    const user = userEvent.setup();
    mockPost.mockResolvedValue({ id: "key-id-1", full_key: "nyxid_ag_secret" });
    const onSuccess = vi.fn();

    render(
      <ApiKeyCreateConfirm
        pairingId={pairingId}
        onSuccess={onSuccess}
        prefill={{
          name: "coding-agent",
          platform: "claude-code",
          scopes: "read write",
        }}
      />,
      { wrapper: createWrapper() },
    );

    // Summary: the CLI-sent name lands in the editable Name field.
    expect(screen.getByLabelText("Name")).toHaveValue("coding-agent");

    await user.click(screen.getByRole("button", { name: /Create Key/i }));

    await waitFor(() => {
      expect(mockReserve).toHaveBeenCalledWith(pairingId);
    });
    // The mint MUST be routed through withRewindOnError(pairingId, fn) so a
    // 4xx rewinds the reservation (duplicate-mint guard). Pin the wrapper is
    // actually invoked — the stub is a pass-through, so without this nothing
    // proves the panel didn't bypass it and call api.post directly.
    expect(mockRewind).toHaveBeenCalledWith(pairingId, expect.any(Function));
    const body = bodyForCall("/api-keys");
    // Default access scope (no allowed_*_csv in prefill) → allow-all,
    // and the scopes string is space-joined from the prefilled set.
    expect(body).toMatchObject({
      name: "coding-agent",
      scopes: "read write",
      platform: "claude-code",
      allow_all_services: true,
      allow_all_nodes: true,
    });
    // allow-all means we do NOT pin specific ids.
    expect(body).not.toHaveProperty("allowed_service_ids");
    expect(body).not.toHaveProperty("allowed_node_ids");

    await waitFor(() => {
      expect(onSuccess).toHaveBeenCalledWith({
        kind: "api-key-create",
        api_key_id: "key-id-1",
        full_key: "nyxid_ag_secret",
      });
    });
  });

  it("derives expires_at from a positive expires_in_days and includes target_org_id when an org is chosen", async () => {
    const user = userEvent.setup();
    mockUseOrgs.mockReturnValue({
      data: [{ id: "org-uuid-1", display_name: "ChronoAI" }],
    });
    mockPost.mockResolvedValue({ id: "key-id-2", full_key: "nyxid_ag_xyz" });

    render(
      <ApiKeyCreateConfirm
        pairingId={pairingId}
        onSuccess={vi.fn()}
        prefill={{ name: "ci-key", expires_in_days: 7 }}
      />,
      { wrapper: createWrapper() },
    );

    await user.selectOptions(screen.getByLabelText("Owner"), "org-uuid-1");
    await user.click(screen.getByRole("button", { name: /Create Key/i }));

    await waitFor(() => expect(mockPost).toHaveBeenCalled());
    const body = bodyForCall("/api-keys");
    expect(body.target_org_id).toBe("org-uuid-1");
    // expires_in_days=7 → an RFC-3339 expires_at roughly a week out, not omitted.
    expect(typeof body.expires_at).toBe("string");
    expect(Date.parse(body.expires_at as string)).toBeGreaterThan(Date.now());
    // platform was never set → omitted.
    expect(body).not.toHaveProperty("platform");
  });

  it("keeps Create disabled when the name is empty (validation-error path) and never calls the API", async () => {
    render(
      <ApiKeyCreateConfirm
        pairingId={pairingId}
        onSuccess={vi.fn()}
        prefill={{ scopes: "read" }}
      />,
      { wrapper: createWrapper() },
    );

    // Empty name fails apiKeyNameSchema → button disabled.
    expect(screen.getByLabelText("Name")).toHaveValue("");
    expect(screen.getByRole("button", { name: /Create Key/i })).toBeDisabled();
    expect(mockPost).not.toHaveBeenCalled();
  });

  it("surfaces a backend error message inline when the mint fails", async () => {
    const user = userEvent.setup();
    mockPost.mockRejectedValue(new Error("name already taken"));

    render(
      <ApiKeyCreateConfirm
        pairingId={pairingId}
        onSuccess={vi.fn()}
        prefill={{ name: "dupe", scopes: "read" }}
      />,
      { wrapper: createWrapper() },
    );

    await user.click(screen.getByRole("button", { name: /Create Key/i }));

    expect(await screen.findByText("name already taken")).toBeInTheDocument();
  });
});

// ── ApiKeyRotateConfirm ──────────────────────────────────────────────

describe("ApiKeyRotateConfirm", () => {
  it("renders the resource name in the summary and POSTs the rotate endpoint, then fires onSuccess", async () => {
    const user = userEvent.setup();
    mockPost.mockResolvedValue({ id: "key-id-9", full_key: "nyxid_ag_new" });
    const onSuccess = vi.fn();

    render(
      <ApiKeyRotateConfirm
        pairingId={pairingId}
        onSuccess={onSuccess}
        prefill={{ resource_id: "key 9/with space", display_name: "prod-agent" }}
      />,
      { wrapper: createWrapper() },
    );

    expect(screen.getByText("prod-agent")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: /Rotate key/i }));

    await waitFor(() => {
      // resource_id is URL-encoded into the path.
      expect(mockPost).toHaveBeenCalledWith(
        "/api-keys/key%209%2Fwith%20space/rotate",
      );
    });
    // Rotate is also routed through the rewind wrapper (same 4xx-rewind /
    // duplicate-mint guard as the create panels). Pin the wrapper invocation.
    expect(mockRewind).toHaveBeenCalledWith(pairingId, expect.any(Function));
    await waitFor(() => {
      expect(onSuccess).toHaveBeenCalledWith({
        kind: "api-key-rotate",
        resource_id: "key-id-9",
        full_key: "nyxid_ag_new",
      });
    });
  });

  it("surfaces a backend error inline when rotation fails (error path)", async () => {
    const user = userEvent.setup();
    mockPost.mockRejectedValue(new Error("key not found"));

    render(
      <ApiKeyRotateConfirm
        pairingId={pairingId}
        onSuccess={vi.fn()}
        prefill={{ resource_id: "key-9", display_name: "prod-agent" }}
      />,
      { wrapper: createWrapper() },
    );

    await user.click(screen.getByRole("button", { name: /Rotate key/i }));
    expect(await screen.findByText("key not found")).toBeInTheDocument();
  });
});

// ── NodeRegisterConfirm ──────────────────────────────────────────────

describe("NodeRegisterConfirm", () => {
  it("POSTs the prefilled node name to /nodes/register-token and fires onSuccess with the token", async () => {
    const user = userEvent.setup();
    mockPost.mockResolvedValue({
      token_id: "tok-id-1",
      token: "nyx_nreg_abc",
      name: "edge-1",
    });
    const onSuccess = vi.fn();

    render(
      <NodeRegisterConfirm
        pairingId={pairingId}
        onSuccess={onSuccess}
        prefill={{ name: "edge-1" }}
      />,
      { wrapper: createWrapper() },
    );

    expect(screen.getByLabelText(/Node name/i)).toHaveValue("edge-1");

    await user.click(screen.getByRole("button", { name: /Generate token/i }));

    await waitFor(() => {
      expect(mockPost).toHaveBeenCalledWith("/nodes/register-token", {
        name: "edge-1",
      });
    });
    await waitFor(() => {
      expect(onSuccess).toHaveBeenCalledWith({
        kind: "node-register-token",
        token_id: "tok-id-1",
        token: "nyx_nreg_abc",
      });
    });
  });

  it("falls back to the my-node default when no name is provided", async () => {
    const user = userEvent.setup();
    mockPost.mockResolvedValue({
      token_id: "tok-id-2",
      token: "nyx_nreg_def",
      name: "my-node",
    });

    render(
      <NodeRegisterConfirm
        pairingId={pairingId}
        onSuccess={vi.fn()}
        prefill={{}}
      />,
      { wrapper: createWrapper() },
    );

    // Blank name is valid (optional) → submit enabled, body uses default.
    await user.click(screen.getByRole("button", { name: /Generate token/i }));
    await waitFor(() => {
      expect(mockPost).toHaveBeenCalledWith("/nodes/register-token", {
        name: "my-node",
      });
    });
  });

  it("disables Generate when the typed name violates the node-name schema (validation path)", async () => {
    const user = userEvent.setup();

    render(
      <NodeRegisterConfirm
        pairingId={pairingId}
        onSuccess={vi.fn()}
        prefill={{}}
      />,
      { wrapper: createWrapper() },
    );

    // Uppercase + spaces are rejected by nodeNameSchema (kebab-case only).
    await user.type(screen.getByLabelText(/Node name/i), "Bad Name");

    expect(
      screen.getByRole("button", { name: /Generate token/i }),
    ).toBeDisabled();
    expect(mockPost).not.toHaveBeenCalled();
  });
});

// ── NodeRotateConfirm ────────────────────────────────────────────────

describe("NodeRotateConfirm", () => {
  it("POSTs the rotate-token endpoint and fires onSuccess with auth_token + signing_secret", async () => {
    const user = userEvent.setup();
    mockPost.mockResolvedValue({
      auth_token: "nyx_nauth_new",
      signing_secret: "sig_new",
    });
    const onSuccess = vi.fn();

    render(
      <NodeRotateConfirm
        pairingId={pairingId}
        onSuccess={onSuccess}
        prefill={{ resource_id: "node-1", display_name: "edge-east" }}
      />,
      { wrapper: createWrapper() },
    );

    expect(screen.getByText("edge-east")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: /Rotate token/i }));

    await waitFor(() => {
      expect(mockPost).toHaveBeenCalledWith("/nodes/node-1/rotate-token");
    });
    await waitFor(() => {
      expect(onSuccess).toHaveBeenCalledWith({
        kind: "node-rotate-token",
        resource_id: "node-1",
        auth_token: "nyx_nauth_new",
        signing_secret: "sig_new",
      });
    });
  });

  it("surfaces a backend error inline when node rotation fails (error path)", async () => {
    const user = userEvent.setup();
    mockPost.mockRejectedValue(new Error("node offline"));

    render(
      <NodeRotateConfirm
        pairingId={pairingId}
        onSuccess={vi.fn()}
        prefill={{ resource_id: "node-1", display_name: "edge-east" }}
      />,
      { wrapper: createWrapper() },
    );

    await user.click(screen.getByRole("button", { name: /Rotate token/i }));
    expect(await screen.findByText("node offline")).toBeInTheDocument();
  });
});

// ── ServiceAccountCreateConfirm ──────────────────────────────────────

describe("ServiceAccountCreateConfirm", () => {
  it("assembles the body (trimmed name/scopes, role_ids list, description, org) and POSTs /admin/service-accounts", async () => {
    const user = userEvent.setup();
    mockUseOrgs.mockReturnValue({
      data: [{ id: "org-uuid-1", display_name: "ChronoAI" }],
    });
    mockPost.mockResolvedValue({
      id: "sa-id-1",
      client_id: "cid-1",
      client_secret: "csecret-1",
    });
    const onSuccess = vi.fn();

    render(
      <ServiceAccountCreateConfirm
        pairingId={pairingId}
        onSuccess={onSuccess}
        prefill={{
          name: "ci-deploys",
          scopes: "openid profile",
          description: "CI bot",
          role_ids_csv: "role-a, role-b",
        }}
      />,
      { wrapper: createWrapper() },
    );

    // Prefilled inputs render the CLI-sent summary values.
    expect(screen.getByLabelText("Name")).toHaveValue("ci-deploys");
    expect(screen.getByLabelText("Allowed scopes")).toHaveValue(
      "openid profile",
    );

    await user.selectOptions(screen.getByLabelText("Owner"), "org-uuid-1");
    await user.click(
      screen.getByRole("button", { name: /Create Service Account/i }),
    );

    await waitFor(() => expect(mockPost).toHaveBeenCalled());
    const body = bodyForCall("/admin/service-accounts");
    expect(body).toMatchObject({
      name: "ci-deploys",
      allowed_scopes: "openid profile",
      description: "CI bot",
      target_org_id: "org-uuid-1",
    });
    // CSV role ids are split + trimmed into an array.
    expect(body.role_ids).toEqual(["role-a", "role-b"]);

    await waitFor(() => {
      expect(onSuccess).toHaveBeenCalledWith({
        kind: "service-account-create",
        service_account_id: "sa-id-1",
        client_id: "cid-1",
        client_secret: "csecret-1",
      });
    });
  });

  it("keeps Create disabled when the name is blank (validation path)", () => {
    render(
      <ServiceAccountCreateConfirm
        pairingId={pairingId}
        onSuccess={vi.fn()}
        prefill={{ scopes: "openid" }}
      />,
      { wrapper: createWrapper() },
    );

    expect(screen.getByLabelText("Name")).toHaveValue("");
    expect(
      screen.getByRole("button", { name: /Create Service Account/i }),
    ).toBeDisabled();
    expect(mockPost).not.toHaveBeenCalled();
  });
});

// ── ServiceAccountRotateSecretConfirm ────────────────────────────────

describe("ServiceAccountRotateSecretConfirm", () => {
  it("POSTs the rotate-secret endpoint and fires onSuccess with the new client_secret", async () => {
    const user = userEvent.setup();
    mockPost.mockResolvedValue({
      client_id: "cid-2",
      client_secret: "csecret-2",
    });
    const onSuccess = vi.fn();

    render(
      <ServiceAccountRotateSecretConfirm
        pairingId={pairingId}
        onSuccess={onSuccess}
        prefill={{ resource_id: "sa-1", display_name: "ci-deploys" }}
      />,
      { wrapper: createWrapper() },
    );

    expect(screen.getByText("ci-deploys")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: /Rotate secret/i }));

    await waitFor(() => {
      expect(mockPost).toHaveBeenCalledWith(
        "/admin/service-accounts/sa-1/rotate-secret",
      );
    });
    await waitFor(() => {
      expect(onSuccess).toHaveBeenCalledWith({
        kind: "service-account-rotate-secret",
        resource_id: "sa-1",
        client_id: "cid-2",
        client_secret: "csecret-2",
      });
    });
  });
});

// ── DeveloperAppCreateConfirm ────────────────────────────────────────

describe("DeveloperAppCreateConfirm", () => {
  it("POSTs /developer/oauth-clients with confidential client_type, scopes array, broker + org, then fires onSuccess", async () => {
    const user = userEvent.setup();
    mockUseOrgs.mockReturnValue({
      data: [{ id: "org-uuid-1", display_name: "ChronoAI" }],
    });
    mockPost.mockResolvedValue({ id: "app-id-1", client_secret: "app-secret-1" });
    const onSuccess = vi.fn();

    render(
      <DeveloperAppCreateConfirm
        pairingId={pairingId}
        onSuccess={onSuccess}
        prefill={{
          name: "Acme Web",
          redirect_uris: ["https://acme.test/cb"],
          allowed_scopes: "openid profile",
          broker_capability: true,
        }}
      />,
      { wrapper: createWrapper() },
    );

    expect(screen.getByLabelText("App name")).toHaveValue("Acme Web");

    await user.selectOptions(screen.getByLabelText("Owner"), "org-uuid-1");
    await user.click(screen.getByRole("button", { name: /Create app/i }));

    await waitFor(() => expect(mockPost).toHaveBeenCalled());
    const body = bodyForCall("/developer/oauth-clients");
    expect(body).toMatchObject({
      name: "Acme Web",
      client_type: "confidential",
      broker_capability_enabled: true,
      target_org_id: "org-uuid-1",
    });
    expect(body.redirect_uris).toEqual(["https://acme.test/cb"]);
    // allowed_scopes is split into a Vec<String>, not a space-string.
    expect(body.allowed_scopes).toEqual(["openid", "profile"]);

    await waitFor(() => {
      expect(onSuccess).toHaveBeenCalledWith({
        kind: "developer-app-create",
        developer_app_id: "app-id-1",
        client_secret: "app-secret-1",
      });
    });
  });

  it("surfaces an inline 'redirect URI required' error and never calls the API when all URIs are blank (validation path)", async () => {
    const user = userEvent.setup();

    render(
      <DeveloperAppCreateConfirm
        pairingId={pairingId}
        onSuccess={vi.fn()}
        prefill={{ name: "Acme Web" }}
      />,
      { wrapper: createWrapper() },
    );

    // Name is set (button enabled) but the single redirect URI row is
    // blank → submit short-circuits with a client-side error.
    await user.click(screen.getByRole("button", { name: /Create app/i }));

    expect(
      await screen.findByText(/At least one redirect URI is required/i),
    ).toBeInTheDocument();
    expect(mockPost).not.toHaveBeenCalled();
  });

  it("supports adding extra redirect URI rows and submits all non-empty ones", async () => {
    const user = userEvent.setup();
    mockPost.mockResolvedValue({ id: "app-id-2", client_secret: "secret-2" });

    render(
      <DeveloperAppCreateConfirm
        pairingId={pairingId}
        onSuccess={vi.fn()}
        prefill={{ name: "Multi", redirect_uris: ["https://one.test/cb"] }}
      />,
      { wrapper: createWrapper() },
    );

    await user.click(
      screen.getByRole("button", { name: /Add redirect URI/i }),
    );
    const uriInputs = screen.getAllByPlaceholderText(
      "https://app.example.com/callback",
    );
    expect(uriInputs).toHaveLength(2);
    await user.type(uriInputs[1]!, "https://two.test/cb");

    await user.click(screen.getByRole("button", { name: /Create app/i }));

    await waitFor(() => expect(mockPost).toHaveBeenCalled());
    expect(bodyForCall("/developer/oauth-clients").redirect_uris).toEqual([
      "https://one.test/cb",
      "https://two.test/cb",
    ]);
  });
});

// ── DeveloperAppRotateSecretConfirm ──────────────────────────────────

describe("DeveloperAppRotateSecretConfirm", () => {
  it("POSTs the rotate-secret endpoint and fires onSuccess with the new client_secret", async () => {
    const user = userEvent.setup();
    mockPost.mockResolvedValue({ id: "app-1", client_secret: "rotated-secret" });
    const onSuccess = vi.fn();

    render(
      <DeveloperAppRotateSecretConfirm
        pairingId={pairingId}
        onSuccess={onSuccess}
        prefill={{ resource_id: "app-1", display_name: "Acme Web" }}
      />,
      { wrapper: createWrapper() },
    );

    expect(screen.getByText("Acme Web")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: /Rotate secret/i }));

    await waitFor(() => {
      expect(mockPost).toHaveBeenCalledWith(
        "/developer/oauth-clients/app-1/rotate-secret",
      );
    });
    await waitFor(() => {
      expect(onSuccess).toHaveBeenCalledWith({
        kind: "developer-app-rotate-secret",
        resource_id: "app-1",
        client_secret: "rotated-secret",
      });
    });
  });
});

// ── MfaSetupConfirm (multi-step: setup → confirm) ────────────────────

describe("MfaSetupConfirm", () => {
  it("runs setup on mount, then confirms the typed code and fires onSuccess with recovery codes (happy path)", async () => {
    const user = userEvent.setup();
    mockPost.mockImplementation(async (path: string) => {
      if (path === "/auth/mfa/setup") {
        return {
          factor_id: "factor-1",
          secret: "JBSWY3DPEHPK3PXP",
          qr_code_url: "otpauth://totp/NyxID:me?secret=JBSWY3DPEHPK3PXP",
        };
      }
      if (path === "/auth/mfa/confirm") {
        return { message: "ok", recovery_codes: ["rc-1", "rc-2"] };
      }
      throw new Error(`unexpected POST ${path}`);
    });
    const onSuccess = vi.fn();

    render(<MfaSetupConfirm pairingId={pairingId} onSuccess={onSuccess} />, {
      wrapper: createWrapper(),
    });

    // Mount fires /auth/mfa/setup; the panel transitions to the
    // QR + code-entry phase.
    await waitFor(() => {
      expect(mockPost).toHaveBeenCalledWith("/auth/mfa/setup", {});
    });
    const codeInput = await screen.findByLabelText(/6-digit code/i);

    await user.type(codeInput, "123456");
    await user.click(
      screen.getByRole("button", { name: /Verify and enable MFA/i }),
    );

    await waitFor(() => {
      expect(mockPost).toHaveBeenCalledWith("/auth/mfa/confirm", {
        code: "123456",
      });
    });
    await waitFor(() => {
      expect(onSuccess).toHaveBeenCalledWith({
        kind: "mfa-setup",
        factor_id: "factor-1",
        recovery_codes: ["rc-1", "rc-2"],
      });
    });
  });

  it("keeps Verify disabled while the code is empty (validation path)", async () => {
    mockPost.mockImplementation(async (path: string) => {
      if (path === "/auth/mfa/setup") {
        return {
          factor_id: "factor-2",
          secret: "SECRET",
          qr_code_url: "otpauth://totp/NyxID:me?secret=SECRET",
        };
      }
      throw new Error(`unexpected POST ${path}`);
    });

    render(<MfaSetupConfirm pairingId={pairingId} onSuccess={vi.fn()} />, {
      wrapper: createWrapper(),
    });

    await screen.findByLabelText(/6-digit code/i);
    // Empty code → Verify is disabled and /auth/mfa/confirm never fires.
    expect(
      screen.getByRole("button", { name: /Verify and enable MFA/i }),
    ).toBeDisabled();
    expect(
      mockPost.mock.calls.some(([p]) => p === "/auth/mfa/confirm"),
    ).toBe(false);
  });

  it("surfaces the setup error and stays in the init phase when /auth/mfa/setup fails (error path)", async () => {
    mockPost.mockRejectedValue(new Error("mfa already enabled"));

    render(<MfaSetupConfirm pairingId={pairingId} onSuccess={vi.fn()} />, {
      wrapper: createWrapper(),
    });

    expect(
      await screen.findByText("mfa already enabled"),
    ).toBeInTheDocument();
    // Still on the "Setting up MFA" init screen — no QR / code entry yet.
    expect(screen.getByText(/Setting up MFA/i)).toBeInTheDocument();
    expect(screen.queryByLabelText(/6-digit code/i)).not.toBeInTheDocument();
  });

  it("re-surfaces a confirm error and lets the user retry (the panel returns to the ready phase)", async () => {
    const user = userEvent.setup();
    mockPost.mockImplementation(async (path: string) => {
      if (path === "/auth/mfa/setup") {
        return {
          factor_id: "factor-3",
          secret: "SECRET",
          qr_code_url: "otpauth://totp/NyxID:me?secret=SECRET",
        };
      }
      if (path === "/auth/mfa/confirm") {
        throw new Error("invalid TOTP code");
      }
      throw new Error(`unexpected POST ${path}`);
    });

    render(<MfaSetupConfirm pairingId={pairingId} onSuccess={vi.fn()} />, {
      wrapper: createWrapper(),
    });

    const codeInput = await screen.findByLabelText(/6-digit code/i);
    await user.type(codeInput, "000000");
    await user.click(
      screen.getByRole("button", { name: /Verify and enable MFA/i }),
    );

    expect(await screen.findByText("invalid TOTP code")).toBeInTheDocument();
    // Back in the "ready" phase: the button reads "Verify..." again, not
    // stuck in "Verifying...".
    const button = screen.getByRole("button", {
      name: /Verify and enable MFA/i,
    });
    expect(button).not.toBeDisabled();
  });
});
