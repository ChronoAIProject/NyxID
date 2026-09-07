import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { ApiKeyDetailPage } from "./api-key-detail";
import type { CredentialSource } from "@/schemas/orgs";

const mocks = vi.hoisted(() => ({
  source: undefined as CredentialSource | undefined,
  revoke: vi.fn(),
}));
vi.mock("@tanstack/react-router", () => ({
  useParams: () => ({ keyId: "key" }),
}));
vi.mock("@/components/layout/dashboard-layout", () => ({
  useBreadcrumbLabel: vi.fn(),
}));
vi.mock("@/hooks/use-api-keys", () => ({
  useApiKey: () => ({
    data: {
      id: "key",
      name: "Agent",
      key_prefix: "nyxid_ag_12345678",
      is_active: true,
      credential_source: mocks.source,
    },
  }),
}));
vi.mock("@/hooks/use-agent-key-login", () => ({
  useLoginCredentials: () => ({
    data: [
      {
        id: "cred",
        label: "Workstation",
        secret_prefix: "nyxid_ag_87654321",
        is_active: true,
        expires_at: null,
        last_used_at: null,
        revoked_reason: null,
        created_at: "2026-01-01T00:00:00Z",
      },
    ],
  }),
  useRevokeLoginCredential: () => ({
    mutateAsync: mocks.revoke,
    reset: vi.fn(),
  }),
}));
vi.mock("@/components/dashboard/api-key-detail/details-card", () => ({
  DetailsCard: () => null,
}));
vi.mock("@/components/dashboard/api-key-detail/node-scope-card", () => ({
  NodeScopeCard: () => null,
}));
vi.mock("@/components/dashboard/api-key-detail/platform-card", () => ({
  PlatformCard: () => null,
}));
vi.mock("@/components/dashboard/api-key-detail/callback-url-card", () => ({
  CallbackUrlCard: () => null,
}));
vi.mock("@/components/dashboard/api-key-detail/rate-limit-card", () => ({
  RateLimitCard: () => null,
}));
vi.mock("@/components/dashboard/api-key-detail/bindings-card", () => ({
  BindingsCard: () => null,
}));
vi.mock("@/components/dashboard/api-key-detail/usage-stats-card", () => ({
  UsageStatsCard: () => null,
}));
vi.mock("@/components/dashboard/api-key-detail/verify-key-card", () => ({
  VerifyKeyCard: () => null,
}));
vi.mock("@/components/dashboard/api-key-detail/rotate-key-dialog", () => ({
  RotateKeyDialog: () => null,
}));
vi.mock("@/components/dashboard/api-key-detail/delete-key-dialog", () => ({
  DeleteKeyDialog: () => null,
}));

afterEach(cleanup);
describe("key detail write access", () => {
  it.each([
    undefined,
    { type: "personal" },
    {
      type: "org",
      org_id: "org",
      org_name: "Team",
      role: "admin",
      allowed: true,
    },
  ] satisfies (CredentialSource | undefined)[])(
    "allows the owner/admin to rotate and revoke keys and credentials (%j)",
    (source) => {
      mocks.source = source;
      render(<ApiKeyDetailPage />);
      expect(
        screen.getByRole("button", { name: "Rotate Key" }),
      ).toBeInTheDocument();
      expect(screen.getAllByRole("button", { name: "Revoke" })).toHaveLength(2);
    },
  );
  it("shows member credential metadata without key or credential mutation actions", () => {
    mocks.source = {
      type: "org",
      org_id: "org",
      org_name: "Team",
      role: "member",
      allowed: true,
    };
    render(<ApiKeyDetailPage />);
    expect(screen.getByText("Workstation")).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "Rotate Key" }),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "Revoke" }),
    ).not.toBeInTheDocument();
    expect(mocks.revoke).not.toHaveBeenCalled();
  });
});
