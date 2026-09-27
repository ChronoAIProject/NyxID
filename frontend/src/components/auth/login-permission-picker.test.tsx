import { useState } from "react";
import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it } from "vitest";
import { LoginPermissionPicker } from "./login-permission-picker";
import type { PermissionOption } from "@/lib/login-permissions";
import type { RequestedPermissions } from "@/schemas/login-request";

const empty: RequestedPermissions = {
  permissions: [],
  services: [],
  service_permissions: [],
};
const options: PermissionOption[] = ["read", "write"].map((value) => ({
  id: `nyxid::${value}`,
  group: "nyxid",
  service: "NyxID",
  label: value,
  description: "NyxID API permission",
  field: "permissions",
  value,
}));

function Harness() {
  const [value, setValue] = useState(empty);
  return (
    <>
      <LoginPermissionPicker
        options={options}
        value={value}
        initial={empty}
        onChange={setValue}
        disabled={false}
      />
      <button type="button">Outside the picker</button>
    </>
  );
}

afterEach(cleanup);

describe("login permission dropdown", () => {
  it("keeps a pointer selection mounted when the input blurs without a focus target", async () => {
    const user = userEvent.setup();
    render(<Harness />);
    const search = screen.getByRole("textbox", { name: "Search permissions" });
    await user.click(search);
    const read = screen.getByRole("checkbox", { name: "NyxID: read" });
    fireEvent.pointerDown(read, { pointerType: "mouse", button: 0 });
    // Safari does not focus buttons on mouse click; the input blur has no relatedTarget.
    fireEvent.blur(search, { relatedTarget: null });
    expect(read).toBeInTheDocument();
    fireEvent.click(read);
    expect(read).toBeChecked();
    expect(screen.getByRole("status")).toHaveTextContent("1 selected");
    expect(
      screen.getByRole("button", { name: "Remove NyxID: read" }),
    ).toBeInTheDocument();
  });

  it("reopens on a search click after Escape and closes on an outside click", async () => {
    const user = userEvent.setup();
    render(<Harness />);
    const search = screen.getByRole("textbox", { name: "Search permissions" });
    await user.click(search);
    await user.keyboard("{Escape}");
    expect(search).toHaveFocus();
    expect(search).toHaveAttribute("aria-expanded", "false");
    await user.click(search);
    expect(search).toHaveAttribute("aria-expanded", "true");
    await user.click(
      screen.getByRole("button", { name: "Outside the picker" }),
    );
    expect(search).toHaveAttribute("aria-expanded", "false");
  });

  it("supports individual and select-all changes through checkbox labels", async () => {
    const user = userEvent.setup();
    render(<Harness />);
    await user.click(
      screen.getByRole("button", { name: "Show permission options" }),
    );
    const group = screen.getByRole("group", { name: "NyxID" });
    await user.click(within(group).getByText("Select all 2"));
    expect(screen.getByRole("status")).toHaveTextContent("2 selected");
    await user.click(within(group).getByText("read", { exact: true }));
    expect(
      screen.getByRole("checkbox", { name: "NyxID: read" }),
    ).not.toBeChecked();
    expect(
      screen.getByRole("checkbox", { name: "NyxID: write" }),
    ).toBeChecked();
    expect(
      screen.getByRole("checkbox", { name: "NyxID: All 2 permissions" }),
    ).toBePartiallyChecked();
    expect(screen.getByRole("status")).toHaveTextContent("1 selected");
    await user.click(within(group).getByText("Select all 2"));
    await user.click(within(group).getByText("Select all 2"));
    expect(screen.getByRole("status")).toHaveTextContent("0 selected");
  });
  it("supports keyboard selection, return to search, and leaving the picker", async () => {
    const user = userEvent.setup();
    render(<Harness />);
    const search = screen.getByRole("textbox", { name: "Search permissions" });
    await user.click(search);
    await user.keyboard("{ArrowDown}");
    await waitFor(() =>
      expect(
        screen.getByRole("checkbox", { name: "NyxID: All 2 permissions" }),
      ).toHaveFocus(),
    );
    await user.tab();
    await user.keyboard(" ");
    expect(screen.getByRole("checkbox", { name: "NyxID: read" })).toBeChecked();
    await user.keyboard("{Escape}");
    expect(search).toHaveFocus();
    expect(search).toHaveAttribute("aria-expanded", "false");
    await user.click(search);
    await user.type(search, "write");
    expect(
      screen.queryByRole("checkbox", { name: "NyxID: read" }),
    ).not.toBeInTheDocument();
    await user.click(screen.getByRole("checkbox", { name: "NyxID: write" }));
    expect(screen.getByRole("status")).toHaveTextContent("2 selected");
    act(() =>
      screen.getByRole("button", { name: "Outside the picker" }).focus(),
    );
    expect(search).toHaveAttribute("aria-expanded", "false");
  });
});
