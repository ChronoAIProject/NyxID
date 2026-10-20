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
      items: [
        {
          id: "bot",
          name: "Team bot",
          owner_user_id: "org",
          owner_name: "Team",
          owner_email: "team@example.test",
        },
        {
          id: "bot-2",
          name: "Other bot",
          owner_user_id: "missing-owner",
          owner_name: null,
        },
      ],
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
  expect(screen.getByText("Team")).toBeVisible();
  expect(screen.getByText("team@example.test")).toBeVisible();
  expect(screen.getByText("org")).toBeVisible();
  expect(screen.getByText("missing-owner")).toBeVisible();
  await userEvent.click(
    screen.getByRole("button", { name: "Transfer ownership of Team bot" }),
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
