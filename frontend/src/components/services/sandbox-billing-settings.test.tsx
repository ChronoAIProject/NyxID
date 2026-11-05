import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { SandboxBillingSettings } from "./sandbox-billing-settings";
const serviceId = "11111111-1111-4111-8111-111111111111";
const producerId = "22222222-2222-4222-8222-222222222222";
const mock = vi.hoisted(() => ({
  save: vi.fn(),
  settings: {
    service_id: "11111111-1111-4111-8111-111111111111",
    revision: 0,
    authorized_producer_service_account_id: null as string | null,
  },
  error: false,
}));
vi.mock("@/hooks/use-sandbox-billing-settings", () => ({
  useSandboxBillingSettings: () => ({
    data: mock.settings,
    isPending: false,
    isError: mock.error,
    refetch: vi.fn(),
  }),
  useUpdateSandboxBillingSettings: () => ({
    mutateAsync: mock.save,
    isPending: false,
  }),
}));
vi.mock("@/hooks/use-service-accounts", () => ({
  useServiceAccounts: () => ({
    data: {
      service_accounts: [
        {
          id: "22222222-2222-4222-8222-222222222222",
          name: "CMA producer",
          purpose: "general",
          is_active: true,
          allowed_scopes: "proxy billing:sandbox:write",
        },
        {
          id: "inactive",
          name: "Inactive",
          purpose: "general",
          is_active: false,
          allowed_scopes: "billing:sandbox:write",
        },
        {
          id: "curator",
          name: "Curator",
          purpose: "curation",
          is_active: true,
          allowed_scopes: "billing:sandbox:write",
        },
        {
          id: "unscoped",
          name: "Unscoped",
          purpose: "general",
          is_active: true,
          allowed_scopes: "proxy",
        },
      ],
    },
    isPending: false,
    isError: false,
  }),
}));
vi.mock("sonner", () => ({ toast: { success: vi.fn() } }));
describe("Sandbox billing producer settings", () => {
  beforeEach(() => {
    mock.error = false;
    mock.settings = {
      service_id: serviceId,
      revision: 0,
      authorized_producer_service_account_id: null,
    };
    mock.save.mockReset();
  });
  it("selects only eligible accounts and saves with the loaded revision", async () => {
    mock.save.mockResolvedValue({
      ...mock.settings,
      revision: 1,
      authorized_producer_service_account_id: producerId,
    });
    render(<SandboxBillingSettings serviceId={serviceId} />);
    const save = screen.getByRole("button", { name: "Save Sandbox producer" });
    expect(save).toBeDisabled();
    expect(
      screen.queryByRole("option", { name: /Inactive|Curator|Unscoped/ }),
    ).not.toBeInTheDocument();
    fireEvent.change(
      screen.getByLabelText("Authorized producer service account"),
      { target: { value: producerId } },
    );
    await waitFor(() => expect(save).toBeEnabled());
    await userEvent.click(save);
    await waitFor(() =>
      expect(mock.save).toHaveBeenCalledWith({
        authorized_producer_service_account_id: producerId,
        expected_revision: 0,
      }),
    );
    await waitFor(() => expect(save).toBeDisabled());
    expect(
      screen.getByRole("link", { name: /Billing Settings/ }),
    ).toHaveAttribute("href", "/admin/billing?tab=settings");
  });
  it("clears explicitly and displays a retained-window refusal", async () => {
    mock.settings = {
      ...mock.settings,
      revision: 4,
      authorized_producer_service_account_id: producerId,
    };
    mock.save.mockRejectedValue(
      new Error(
        "Submit evidence for funded Sandbox windows before changing the producer",
      ),
    );
    render(<SandboxBillingSettings serviceId={serviceId} />);
    fireEvent.change(
      screen.getByLabelText("Authorized producer service account"),
      { target: { value: "" } },
    );
    await userEvent.click(
      screen.getByRole("button", { name: "Save Sandbox producer" }),
    );
    await waitFor(() =>
      expect(mock.save).toHaveBeenCalledWith({
        authorized_producer_service_account_id: null,
        expected_revision: 4,
      }),
    );
    expect(
      await screen.findByText(
        /Submit evidence for funded Sandbox windows before changing/,
      ),
    ).toBeInTheDocument();
  });
  it("blocks a stale draft until the changed settings are reloaded", async () => {
    const view = render(<SandboxBillingSettings serviceId={serviceId} />);
    fireEvent.change(
      screen.getByLabelText("Authorized producer service account"),
      { target: { value: producerId } },
    );
    mock.settings = { ...mock.settings, revision: 1 };
    view.rerender(<SandboxBillingSettings serviceId={serviceId} />);
    expect(
      screen.getByRole("button", { name: "Save Sandbox producer" }),
    ).toBeDisabled();
    expect(mock.save).not.toHaveBeenCalled();
  });
  it("does not offer an editor when settings cannot be loaded", () => {
    mock.error = true;
    render(<SandboxBillingSettings serviceId={serviceId} />);
    expect(screen.getByRole("alert")).toHaveTextContent(
      "Could not load Sandbox settings",
    );
    expect(
      screen.queryByRole("button", { name: "Save Sandbox producer" }),
    ).not.toBeInTheDocument();
  });
});
