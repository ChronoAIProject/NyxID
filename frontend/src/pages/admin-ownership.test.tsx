import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { expect, it, vi } from "vitest";
import { AdminOwnershipPage } from "./admin-ownership";
const mock = vi.hoisted(() => ({ allowed: true }));
vi.mock("@/stores/auth-store", () => ({
  useAuthStore: (select: (state: unknown) => unknown) =>
    select({
      user: {
        id: "actor",
        is_admin: mock.allowed,
        role: mock.allowed ? "admin" : "user",
      },
    }),
}));
vi.mock("@/hooks/use-ownership-transfers", () => ({
  useOwnershipResources: () => ({
    data: {
      items: [{ id: "bot", name: "Team bot", owner_user_id: "org" }],
      next_offset: null,
    },
  }),
}));
vi.mock("@/components/shared/ownership-transfer-dialog", () => ({
  OwnershipTransferDialog: () => (
    <div role="dialog">Shared transfer review</div>
  ),
}));
it("retains the admin inventory and opens the shared transfer dialog", async () => {
  mock.allowed = true;
  render(<AdminOwnershipPage />);
  await userEvent.click(
    screen.getByRole("button", { name: /Transfer ownership/ }),
  );
  expect(screen.getByRole("dialog")).toHaveTextContent(
    "Shared transfer review",
  );
});
it("directs owners to asset settings instead of exposing the admin inventory", () => {
  mock.allowed = false;
  render(<AdminOwnershipPage />);
  expect(screen.getByText(/Owners can transfer their assets/)).toBeVisible();
  expect(
    screen.queryByRole("button", { name: /Transfer ownership/ }),
  ).not.toBeInTheDocument();
});
