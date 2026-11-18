import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, expect, it, vi } from "vitest";
import { AutomationDateTime } from "./automation-date-time";
import { TimezoneSelect } from "./timezone-select";
import { GroupedList } from "@/components/ui/grouped-list";

afterEach(cleanup);
it("links a timezone validation error outside the grouped row", () => {
  render(
    <GroupedList footer={<p id="timezone-error">Choose a valid timezone.</p>}>
      <TimezoneSelect
        grouped
        value="Mars/Olympus"
        onChange={vi.fn()}
        describedBy="timezone-error"
        invalid
      />
    </GroupedList>,
  );
  expect(screen.getByRole("combobox", { name: "Timezone" })).toHaveAttribute("aria-invalid", "true");
  expect(screen.getByRole("combobox", { name: "Timezone" })).toHaveAccessibleDescription("Choose a valid timezone.");
});

it.each([false, true])(
  "preserves local date/time edits and Clear with grouped=%s",
  async (grouped) => {
    const change = vi.fn();
    const control = (
      <AutomationDateTime
        label="Run at"
        value="2026-11-05T09:30"
        timezone="Asia/Singapore"
        optional
        onChange={change}
        grouped={grouped}
      />
    );
    render(grouped ? <GroupedList>{control}</GroupedList> : control);
    expect(
      screen.getByRole("button", { name: "Run at date" }),
    ).toHaveTextContent("Nov");
    fireEvent.change(screen.getByLabelText("Run at time"), {
      target: { value: "10:15" },
    });
    expect(change).toHaveBeenLastCalledWith("2026-11-05T10:15");
    await userEvent.click(screen.getByRole("button", { name: "Clear Run at" }));
    expect(change).toHaveBeenLastCalledWith("");
    expect(screen.getByLabelText("Run at time")).toHaveClass(
      grouped ? "border-0" : "border",
    );
  },
);
it.each([false, true])(
  "preserves searchable timezone selection with grouped=%s",
  async (grouped) => {
    const change = vi.fn();
    const control = (
      <TimezoneSelect
        value="Asia/Singapore"
        onChange={change}
        grouped={grouped}
      />
    );
    render(grouped ? <GroupedList>{control}</GroupedList> : control);
    await userEvent.click(screen.getByRole("combobox", { name: "Timezone" }));
    await userEvent.type(
      screen.getByRole("textbox", { name: "Search timezones" }),
      "Europe/London",
    );
    await userEvent.click(
      screen.getByRole("option", { name: /Europe\/London/ }),
    );
    expect(change).toHaveBeenCalledExactlyOnceWith("Europe/London");
    expect(screen.queryByRole("listbox")).not.toBeInTheDocument();
  },
);
