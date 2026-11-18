import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { expect, it, vi } from "vitest";
import {
  GroupedDisclosure,
  GroupedFooter,
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

it("places portaled form descriptions and validation outside the card with linked IDs", async () => {
  render(
    <GroupedList title="Profile">
      <GroupedRow>
        <label htmlFor="portaled-name">Name</label>
        <GroupedInput
          id="portaled-name"
          aria-describedby="name-help portaled-error"
          aria-invalid
        />
        <GroupedFooter>
          <p id="name-help">Shown to everyone.</p>
          <p id="portaled-error" role="alert">
            Name is required.
          </p>
        </GroupedFooter>
      </GroupedRow>
    </GroupedList>,
  );
  const input = screen.getByRole("textbox", { name: "Name" });
  expect(input).toHaveAccessibleDescription(
    "Shown to everyone. Name is required.",
  );
  const error = screen.getByRole("alert");
  expect(error.closest("[data-grouped-list]")).toBeNull();
  expect(error.closest("[data-grouped-footer]")).toBeInTheDocument();
});

it("merges native child classes so destructive action rows override the default text color", () => {
  render(<GroupedList><GroupedRow asChild interactive><button className="text-destructive">Disconnect account</button></GroupedRow></GroupedList>);
  const action = screen.getByRole("button", { name: "Disconnect account" });
  expect(action).toHaveClass("text-destructive");
  expect(action).not.toHaveClass("text-foreground");
});
