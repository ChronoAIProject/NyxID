import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, it, vi } from "vitest";
import { OwnershipTransferCard } from "./ownership-transfer-card";

const mock = vi.hoisted(() => ({
  user: { id: "admin", is_admin: true, role: "admin" },
  dialog: vi.fn(),
  allowed: true,
}));
vi.mock("@/stores/auth-store", () => ({
  useAuthStore: (selector: (state: { user: typeof mock.user }) => unknown) =>
    selector({ user: mock.user }),
}));
vi.mock("@/hooks/use-ownership-transfers", () => ({
  useOwnershipTransferAuthorization: () => ({
    data: { can_transfer: mock.allowed },
  }),
}));
vi.mock("./ownership-transfer-dialog", () => ({
  OwnershipTransferDialog: (props: unknown) => {
    mock.dialog(props);
    return <div role="dialog">Review ownership transfer</div>;
  },
}));
const resource = {
  id: "service",
  name: "Service",
  owner_user_id: "org",
  slug: "service",
  platform: null,
};
beforeEach(() => {
  vi.clearAllMocks();
  mock.allowed = true;
  mock.user = { id: "admin", is_admin: true, role: "admin" };
});

it.each(["operator", "user"])(
  "hides the transfer card from %s accounts",
  (role) => {
    mock.user = { id: "user", is_admin: false, role };
    mock.allowed = false;
    render(<OwnershipTransferCard kind="service" resource={resource} />);
    expect(screen.queryByText("Ownership transfer")).not.toBeInTheDocument();
    expect(mock.dialog).not.toHaveBeenCalled();
  },
);

it("opens the review for the current resource and discards it on resource or identity changes", async () => {
  const user = userEvent.setup();
  const view = render(
    <OwnershipTransferCard kind="service" resource={resource} />,
  );
  await user.click(screen.getByRole("button", { name: "Transfer ownership" }));
  expect(mock.dialog).toHaveBeenLastCalledWith(
    expect.objectContaining({ kind: "service", resource }),
  );
  view.rerender(
    <OwnershipTransferCard
      kind="service"
      resource={{ ...resource, id: "another-service" }}
    />,
  );
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  await user.click(screen.getByRole("button", { name: "Transfer ownership" }));
  mock.user = { ...mock.user, id: "another-admin" };
  view.rerender(
    <OwnershipTransferCard
      kind="service"
      resource={{ ...resource, id: "another-service" }}
    />,
  );
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
});

it.each(["owner", "org-admin"])(
  "shows the card to an authorized %s without a platform role",
  (id) => {
    mock.user = { id, is_admin: false, role: "user" };
    render(<OwnershipTransferCard kind="service" resource={resource} />);
    expect(
      screen.getByRole("button", { name: "Transfer ownership" }),
    ).toBeVisible();
  },
);
