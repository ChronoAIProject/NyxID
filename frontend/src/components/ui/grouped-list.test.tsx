import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { expect, it, vi } from "vitest";
import {
  GroupedDisclosure,
  GroupedInput,
  GroupedList,
  GroupedRow,
} from "./grouped-list";

it("keeps native row semantics, names and disabled behavior", async () => {
  const user = userEvent.setup();
  const open = vi.fn();
  render(
    <GroupedList title="Access" footer="Only the owner can change this.">
      <GroupedDisclosure onClick={open}>Service operations</GroupedDisclosure>
      <GroupedRow asChild interactive>
        <button disabled onClick={open}>
          Unavailable machine
        </button>
      </GroupedRow>
      <GroupedRow
        title="Account"
        accessory="Read only"
        description="Look up connected services"
      />
    </GroupedList>,
  );
  expect(screen.getByRole("region", { name: "Access" })).toHaveTextContent(
    "Read only",
  );
  await user.click(screen.getByRole("button", { name: "Service operations" }));
  await user.click(screen.getByRole("button", { name: "Unavailable machine" }));
  expect(open).toHaveBeenCalledOnce();
});

it("preserves labels and error descriptions for borderless controls", async () => {
  render(
    <GroupedList
      footer={
        <p id="name-error" role="alert">
          Choose a name.
        </p>
      }
    >
      <GroupedRow>
        <label htmlFor="name">Name</label>
        <GroupedInput id="name" aria-invalid aria-describedby="name-error" />
      </GroupedRow>
    </GroupedList>,
  );
  const input = screen.getByRole("textbox", { name: "Name" });
  expect(input).toHaveAccessibleDescription("Choose a name.");
  expect(input).toBeInvalid();
  await userEvent.setup().type(input, "Scout");
  expect(input).toHaveValue("Scout");
});
