import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, it, vi } from "vitest";
import { KeyReadGrantSection } from "./key-read-grant-section";
import type { KeyReadGrant } from "@/hooks/use-sa-key-read-grant";

const state = vi.hoisted(() => ({
  grant: null as KeyReadGrant | null,
  save: vi.fn(),
  revoke: vi.fn(),
  isAdmin: true,
}));

vi.mock("@/hooks/use-sa-key-read-grant", () => ({
  useSaKeyReadGrant: () => ({ data: state.grant, isLoading: false, error: null }),
  useSaveSaKeyReadGrant: () => ({ mutateAsync: state.save, isPending: false }),
  useRevokeSaKeyReadGrant: () => ({ mutateAsync: state.revoke, isPending: false }),
}));
vi.mock("@/stores/auth-store", () => ({
  useAuthStore: (selector: (value: { user: { is_admin: boolean } }) => unknown) => selector({ user: { is_admin: state.isAdmin } }),
}));

beforeEach(() => {
  state.grant = null;
  state.isAdmin = true;
  state.save.mockReset();
  state.revoke.mockReset();
});

it("grants exact connection IDs and removes displayed access after revocation", async () => {
  const user = userEvent.setup();
  const id = "da36ec15-a0bc-4249-917a-9fcedd71bc62";
  state.save.mockImplementation(async (data) => {
    state.grant = { ...data, service_account_id: "sa-1", issued_by: "admin", issued_at: "2026-09-01T00:00:00Z", expires_at: null };
    return state.grant;
  });
  const view = render(<KeyReadGrantSection saId="sa-1" />);
  expect(screen.getByText(/user-services:read/)).toBeInTheDocument();
  await user.click(screen.getByRole("button", { name: "Grant read access" }));
  await user.type(screen.getByLabelText("User-service UUIDs"), ` ${id} `);
  await waitFor(() => expect(screen.getByRole("button", { name: "Save read grant" })).toBeEnabled());
  await user.click(screen.getByRole("button", { name: "Save read grant" }));
  await waitFor(() => expect(state.save).toHaveBeenCalledExactlyOnceWith({ user_service_ids: [id], expires_at: undefined }));
  expect(await screen.findByText(id)).toBeInTheDocument();
  state.revoke.mockImplementation(async () => { state.grant = null; });
  await user.click(screen.getByRole("button", { name: "Revoke read access" }));
  await waitFor(() => expect(state.revoke).toHaveBeenCalledOnce());
  view.rerender(<KeyReadGrantSection saId="sa-1" />);
  expect(screen.queryByText(id)).not.toBeInTheDocument();
});

it("rejects a slug and only exposes grant controls to platform admins", async () => {
  const user = userEvent.setup();
  const view = render(<KeyReadGrantSection saId="sa-1" />);
  await user.click(screen.getByRole("button", { name: "Grant read access" }));
  fireEvent.change(screen.getByLabelText("User-service UUIDs"), { target: { value: "ornn-api" } });
  await waitFor(() => expect(screen.getByRole("button", { name: "Save read grant" })).toBeDisabled());
  expect(state.save).not.toHaveBeenCalled();
  state.isAdmin = false;
  view.rerender(<KeyReadGrantSection saId="sa-1" />);
  expect(screen.queryByRole("button", { name: "Save read grant" })).not.toBeInTheDocument();
  expect(screen.queryByRole("button", { name: "Grant read access" })).not.toBeInTheDocument();
});
