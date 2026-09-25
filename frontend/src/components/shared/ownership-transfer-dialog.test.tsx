import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, it, vi } from "vitest";
import { OwnershipTransferDialog } from "./ownership-transfer-dialog";
import { ApiError } from "@/lib/api-client";

const mock = vi.hoisted(() => ({
  preview: vi.fn(),
  transfer: vi.fn(),
  close: vi.fn(),
}));
const destination = "52f82de8-217b-4666-9bfc-512e684f781d";
const resource = {
  id: "1e76817e-586e-46af-817f-df1ed1af29cd",
  name: "Team bot",
  owner_user_id: "previous-owner",
  platform: "telegram",
  slug: null,
};
const preview = {
  resource_kind: "channel_bot",
  resource_id: resource.id,
  name: resource.name,
  previous_owner_user_id: resource.owner_user_id,
  new_owner_user_id: destination,
  destination_name: "Operations",
  destination_type: "org",
  version: "a".repeat(64),
  routes_to_retire: 2,
  blockers: [],
  effects: ["Old routes will be retired."],
};

vi.mock("@/hooks/use-ownership-transfers", () => ({
  useOwnershipDestinations: () => ({
    data: {
      users: [
        {
          id: destination,
          display_name: "Operations",
          email: "ops@example.com",
          is_active: true,
        },
      ],
      total: 1,
    },
    isLoading: false,
  }),
  useOwnershipTransferPreview: () => ({
    mutateAsync: mock.preview,
    isPending: false,
  }),
  useOwnershipTransfer: () => ({
    mutateAsync: mock.transfer,
    isPending: false,
  }),
}));
beforeEach(() => {
  vi.clearAllMocks();
  mock.preview.mockResolvedValue(preview);
  mock.transfer.mockResolvedValue({ transfer_id: "receipt" });
});

async function reviewTransfer() {
  const user = userEvent.setup();
  await user.click(screen.getByRole("combobox", { name: "Destination owner" }));
  await user.click(await screen.findByRole("option", { name: /Operations/ }));
  await user.click(screen.getByRole("button", { name: "Review transfer" }));
  await screen.findByText("Review ownership transfer");
  return user;
}

it("previews before writing and submits the reviewed destination and version", async () => {
  render(
    <OwnershipTransferDialog
      kind="channel_bot"
      resource={resource}
      onClose={mock.close}
    />,
  );
  const user = await reviewTransfer();
  expect(mock.preview).toHaveBeenCalledWith(destination);
  expect(mock.transfer).not.toHaveBeenCalled();
  expect(
    screen.getByText("2 routes will be permanently retired."),
  ).toBeInTheDocument();
  await user.click(screen.getByRole("button", { name: "Confirm transfer" }));
  await waitFor(() =>
    expect(mock.transfer).toHaveBeenCalledWith({
      new_owner_user_id: destination,
      expected_version: preview.version,
      request_id: expect.any(String),
    }),
  );
  expect(mock.close).toHaveBeenCalledOnce();
});

it("cancelling the review preserves the destination and never transfers", async () => {
  render(
    <OwnershipTransferDialog
      kind="channel_bot"
      resource={resource}
      onClose={mock.close}
    />,
  );
  const user = await reviewTransfer();
  await user.click(screen.getByRole("button", { name: "Back" }));
  expect(
    screen.getByRole("combobox", { name: "Destination owner" }),
  ).toHaveTextContent("Operations");
  await user.click(screen.getByRole("button", { name: "Cancel" }));
  expect(mock.close).toHaveBeenCalledOnce();
  expect(mock.transfer).not.toHaveBeenCalled();
});

it("shows blockers and prevents confirmation", async () => {
  mock.preview.mockResolvedValue({
    ...preview,
    blockers: ["This bot requires OAuth reconnection."],
  });
  render(
    <OwnershipTransferDialog
      kind="channel_bot"
      resource={resource}
      onClose={mock.close}
    />,
  );
  await reviewTransfer();
  expect(screen.getByRole("alert")).toHaveTextContent(
    "This bot requires OAuth reconnection.",
  );
  expect(
    screen.getByRole("button", { name: "Confirm transfer" }),
  ).toBeDisabled();
  expect(mock.transfer).not.toHaveBeenCalled();
});

it("retains the request ID after an uncertain failure and suppresses duplicate submits", async () => {
  let fail!: (error: Error) => void;
  mock.transfer.mockImplementationOnce(
    () =>
      new Promise((_, reject) => {
        fail = reject;
      }),
  );
  render(
    <OwnershipTransferDialog
      kind="channel_bot"
      resource={resource}
      onClose={mock.close}
    />,
  );
  const user = await reviewTransfer();
  const confirm = screen.getByRole("button", { name: "Confirm transfer" });
  fireEvent.click(confirm);
  fireEvent.click(confirm);
  expect(mock.transfer).toHaveBeenCalledOnce();
  fail(new Error("Response lost. Retry the transfer."));
  await screen.findByText("Response lost. Retry the transfer.");
  await user.click(confirm);
  expect(mock.transfer).toHaveBeenCalledTimes(2);
  expect(mock.transfer.mock.calls[1]).toEqual(mock.transfer.mock.calls[0]);
});

it("requires a new preview after a conflict while preserving the chosen owner", async () => {
  mock.transfer.mockRejectedValueOnce(
    new ApiError(409, {
      message: "Resource changed. Review again.",
      error_code: 1003,
      error: "conflict",
    }),
  );
  render(
    <OwnershipTransferDialog
      kind="channel_bot"
      resource={resource}
      onClose={mock.close}
    />,
  );
  const user = await reviewTransfer();
  await user.click(screen.getByRole("button", { name: "Confirm transfer" }));
  await screen.findByText("Resource changed. Review again.");
  expect(
    screen.queryByRole("button", { name: "Confirm transfer" }),
  ).not.toBeInTheDocument();
  expect(
    screen.getByRole("combobox", { name: "Destination owner" }),
  ).toHaveTextContent("Operations");
  expect(screen.getByRole("button", { name: "Review transfer" })).toBeEnabled();
});
