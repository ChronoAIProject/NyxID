import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { ApiError } from "@/lib/api-client";
import type { ServiceAccount } from "@/types/service-accounts";

const mocks = vi.hoisted(() => ({
  isAdmin: true,
  issue: vi.fn(),
  revoke: vi.fn(),
  success: vi.fn(),
  error: vi.fn(),
}));

vi.mock("@/hooks/use-service-accounts", () => ({
  useIssueCurationGrant: () => ({ mutateAsync: mocks.issue, isPending: false }),
  useRevokeCurationGrant: () => ({ mutateAsync: mocks.revoke, isPending: false }),
}));
vi.mock("@/stores/auth-store", () => ({
  useAuthStore: (selector: (state: { user: { is_admin: boolean } }) => unknown) =>
    selector({ user: { is_admin: mocks.isAdmin } }),
}));
vi.mock("sonner", () => ({
  toast: { success: mocks.success, error: mocks.error },
}));

import { CurationGrantSection } from "./curation-grant-section";

const firstService = "f6d0788d-504a-4827-b06a-e40e3df57d62";
const secondService = "45f27ef1-9797-47ee-95a1-a2cc64954d90";
const ornnService = "16214374-87a0-4a81-bd01-ebfb798f6288";
const account: ServiceAccount = {
  id: "editor-account",
  name: "Aevatar editor",
  description: null,
  client_id: "sa_editor",
  secret_prefix: "sas_test",
  role_ids: [],
  allowed_scopes: "catalog:skills:read catalog:skills:write",
  is_active: true,
  rate_limit_override: null,
  created_by: "admin",
  created_at: "2026-09-16T00:00:00Z",
  updated_at: "2026-09-16T00:00:00Z",
  last_authenticated_at: null,
  purpose: "general",
  platform_protected: false,
  credential_generation: 0,
  curation_grant: null,
};

const grantedAccount: ServiceAccount = {
  ...account,
  purpose: "curation",
  platform_protected: true,
  curation_grant: {
    id: "grant-1",
    service_ids: [firstService],
    ornn_proxy_service_id: ornnService,
    issued_by: "admin",
    issued_at: "2026-09-16T00:00:00Z",
    expires_at: null,
    max_writes: 20,
    window_seconds: 3600,
    window_started_at: "2026-09-16T00:00:00Z",
    writes_used: 3,
  },
};

describe("CurationGrantSection", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mocks.isAdmin = true;
    mocks.issue.mockResolvedValue(grantedAccount);
    mocks.revoke.mockResolvedValue({ ...grantedAccount, curation_grant: null });
  });

  it("issues an exact service grant with validated limits through the admin form", async () => {
    const user = userEvent.setup();
    render(<CurationGrantSection account={account} />);
    await user.click(screen.getByRole("button", { name: "Issue grant" }));
    expect(screen.getByRole("button", { name: "Issue grant" })).toBeDisabled();

    await user.type(screen.getByLabelText("Catalog service UUIDs"), `${firstService}, ${secondService}`);
    await user.type(screen.getByLabelText("Ornn catalog service UUID (optional)"), ornnService);
    await waitFor(() => expect(screen.getByRole("button", { name: "Issue grant" })).toBeEnabled());
    await user.click(screen.getByRole("button", { name: "Issue grant" }));

    await waitFor(() => expect(mocks.issue).toHaveBeenCalledWith({
      saId: account.id,
      data: {
        service_ids: [firstService, secondService],
        ornn_proxy_service_id: ornnService,
        expires_at: undefined,
        max_writes: 100,
        window_seconds: 3600,
      },
    }));
    expect(mocks.success).toHaveBeenCalledWith("Curation grant issued");
  });

  it("prevents duplicate resources and out-of-range write limits", async () => {
    const user = userEvent.setup();
    render(<CurationGrantSection account={account} />);
    await user.click(screen.getByRole("button", { name: "Issue grant" }));
    const ids = screen.getByLabelText("Catalog service UUIDs");
    await user.type(ids, `${firstService}, ${firstService}`);
    expect(screen.getByRole("button", { name: "Issue grant" })).toBeDisabled();

    await user.clear(ids);
    await user.type(ids, firstService);
    const limit = screen.getByLabelText("Maximum writes per window");
    await user.clear(limit);
    await user.type(limit, "10001");
    expect(screen.getByRole("button", { name: "Issue grant" })).toBeDisabled();
    expect(mocks.issue).not.toHaveBeenCalled();
  });

  it("retains the submitted grant for retry after a server rejection", async () => {
    const user = userEvent.setup();
    mocks.issue.mockRejectedValueOnce(new ApiError(409, {
      error: "conflict",
      error_code: 409,
      message: "Service account changed; reload before issuing grant",
    }));
    render(<CurationGrantSection account={account} />);
    await user.click(screen.getByRole("button", { name: "Issue grant" }));
    await user.type(screen.getByLabelText("Catalog service UUIDs"), firstService);
    await waitFor(() => expect(screen.getByRole("button", { name: "Issue grant" })).toBeEnabled());
    await user.click(screen.getByRole("button", { name: "Issue grant" }));

    expect(await screen.findByRole("alert")).toHaveTextContent("Service account changed");
    expect(screen.getByLabelText("Catalog service UUIDs")).toHaveValue(firstService);
    expect(mocks.success).not.toHaveBeenCalled();
    await waitFor(() => expect(screen.getByRole("button", { name: "Issue grant" })).toBeEnabled());
    await user.click(screen.getByRole("button", { name: "Issue grant" }));
    await waitFor(() => expect(mocks.issue).toHaveBeenCalledTimes(2));
  });

  it("revokes a grant while continuing to show protected custody", async () => {
    const user = userEvent.setup();
    const view = render(<CurationGrantSection account={grantedAccount} />);
    await user.click(screen.getByRole("button", { name: "Revoke grant" }));
    await waitFor(() => expect(mocks.revoke).toHaveBeenCalledWith(account.id));
    view.rerender(<CurationGrantSection account={{ ...grantedAccount, curation_grant: null }} />);
    expect(screen.getByText("Platform admin only")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Revoke grant" })).not.toBeInTheDocument();
  });

  it("shows a grant to an operator without offering management actions", () => {
    mocks.isAdmin = false;
    render(<CurationGrantSection account={grantedAccount} />);
    expect(screen.getByText(firstService)).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /issue grant|replace grant|revoke grant/i })).not.toBeInTheDocument();
  });
});
