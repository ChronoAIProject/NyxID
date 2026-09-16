import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { expect, it, vi } from "vitest";
import { ServiceAccountDetail } from "./service-account-detail";
const mock = vi.hoisted(() => ({
  update: vi.fn(),
  account: {
    id: "sa-1",
    name: "Worker",
    description: "Saved",
    client_id: "client-1",
    secret_prefix: "prefix",
    allowed_scopes: "openid",
    role_ids: ["role-a"],
    rate_limit_override: 10,
    is_active: true,
    created_at: "2026-09-01T00:00:00Z",
    updated_at: "2026-09-01T00:00:00Z",
    last_authenticated_at: null,
  },
}));
vi.mock("@tanstack/react-router", () => ({
  useNavigate: () => vi.fn(),
  useSearch: () => ({}),
}));
vi.mock("@/hooks/use-service-accounts", () => ({
  useServiceAccount: () => ({ data: mock.account, isLoading: false }),
  useUpdateServiceAccount: () => ({
    mutateAsync: mock.update,
    isPending: false,
  }),
  useDeleteServiceAccount: () => ({ mutateAsync: vi.fn(), isPending: false }),
  useRotateSecret: () => ({ mutateAsync: vi.fn(), isPending: false }),
  useRevokeTokens: () => ({ mutateAsync: vi.fn(), isPending: false }),
}));
vi.mock("@/components/dashboard/sa-connected-services", () => ({
  SaConnectedServices: () => null,
}));
it("does not restore revoked permissions after a refresh when renaming an account", async () => {
  const user = userEvent.setup();
  const element = (
    <ServiceAccountDetail
      saId="sa-1"
      backTo={{ to: "/admin/service-accounts", label: "Accounts" }}
      showProviderSections={false}
    />
  );
  const view = render(element);
  await user.click(screen.getByRole("button", { name: "Edit" }));
  mock.account = { ...mock.account, is_active: false, role_ids: [] };
  view.rerender(
    <ServiceAccountDetail
      saId="sa-1"
      backTo={{ to: "/admin/service-accounts", label: "Accounts" }}
      showProviderSections={false}
    />,
  );
  fireEvent.change(screen.getByLabelText("Name"), {
    target: { value: "Renamed worker" },
  });
  await user.click(screen.getByRole("button", { name: "Save Changes" }));
  expect(mock.update).not.toHaveBeenCalled();
  await user.click(
    await screen.findByRole("button", { name: "Confirm changes" }),
  );
  await waitFor(() =>
    expect(mock.update).toHaveBeenCalledExactlyOnceWith({
      saId: "sa-1",
      data: { name: "Renamed worker" },
    }),
  );
});
