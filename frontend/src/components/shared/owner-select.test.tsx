import { useState } from "react";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, it, vi } from "vitest";
import { OwnerSelect, type SelectedOwner } from "./owner-select";

const mock = vi.hoisted(() => ({
  query: vi.fn(),
  refetch: vi.fn(),
  error: null as Error | null,
}));
const owner = {
  id: "destination",
  display_name: "Calvin",
  email: "calvin@example.test",
  is_active: true,
};
vi.mock("@/hooks/use-ownership-transfers", () => ({
  useOwnershipDestinations: (...args: unknown[]) => {
    mock.query(...args);
    return {
      data: {
        users:
          args[2] === 2
            ? [owner]
            : [
                { ...owner, id: "current", display_name: "Current" },
                {
                  ...owner,
                  id: "inactive",
                  display_name: "Inactive",
                  is_active: false,
                },
                owner,
              ],
        total: 21,
      },
      isFetching: false,
      error: mock.error,
      refetch: mock.refetch,
    };
  },
}));
function Picker() {
  const [value, setValue] = useState<SelectedOwner | null>(null);
  return (
    <OwnerSelect
      id="owner"
      kind="service"
      resourceId="resource"
      ownerType="person"
      currentOwnerId="current"
      value={value}
      onChange={setValue}
    />
  );
}
beforeEach(() => {
  vi.clearAllMocks();
  mock.error = null;
});

it("searches within the dropdown and selects with the keyboard", async () => {
  const user = userEvent.setup();
  render(<Picker />);
  expect(
    screen.queryByRole("combobox", { name: "Search owners" }),
  ).not.toBeInTheDocument();
  await user.click(screen.getByRole("combobox", { name: "Destination owner" }));
  await user.type(
    screen.getByRole("combobox", { name: "Search owners" }),
    "calvin",
  );
  await waitFor(() =>
    expect(mock.query).toHaveBeenLastCalledWith(
      "service",
      "resource",
      1,
      "calvin",
      "person",
      true,
    ),
  );
  expect(screen.getByRole("option", { name: /Current/ })).toHaveAttribute(
    "aria-disabled",
    "true",
  );
  expect(screen.getByRole("option", { name: /Inactive/ })).toHaveAttribute(
    "aria-disabled",
    "true",
  );
  await user.keyboard("{ArrowDown}{Enter}");
  expect(
    screen.getByRole("combobox", { name: "Destination owner" }),
  ).toHaveTextContent("Calvin");
  expect(
    screen.queryByRole("combobox", { name: "Search owners" }),
  ).not.toBeInTheDocument();
});

it("paginates results inside the dropdown and preserves the selection on reopen", async () => {
  const user = userEvent.setup();
  render(<Picker />);
  const trigger = screen.getByRole("combobox", { name: "Destination owner" });
  await user.click(trigger);
  await user.click(screen.getByRole("button", { name: "More owners" }));
  expect(mock.query).toHaveBeenLastCalledWith(
    "service",
    "resource",
    2,
    "",
    "person",
    true,
  );
  await user.click(screen.getByRole("option", { name: /Calvin/ }));
  await user.click(trigger);
  expect(trigger).toHaveTextContent("calvin@example.test");
  await user.type(
    screen.getByRole("combobox", { name: "Search owners" }),
    "new",
  );
  await waitFor(() =>
    expect(mock.query).toHaveBeenLastCalledWith(
      "service",
      "resource",
      1,
      "new",
      "person",
      true,
    ),
  );
});

it("offers retry without selectable stale results after an owner lookup fails", async () => {
  mock.error = new Error("offline");
  const user = userEvent.setup();
  render(<Picker />);
  await user.click(screen.getByRole("combobox", { name: "Destination owner" }));
  expect(screen.getByRole("alert")).toHaveTextContent("Could not load owners");
  expect(screen.queryAllByRole("option")).toHaveLength(0);
  await user.click(screen.getByRole("button", { name: "Retry" }));
  expect(mock.refetch).toHaveBeenCalledOnce();
});
