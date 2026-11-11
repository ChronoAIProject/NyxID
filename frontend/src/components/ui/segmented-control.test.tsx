import { useState } from "react";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { expect, it } from "vitest";
import {
  SegmentedControl,
  SegmentedControlContent,
  SegmentedControlList,
  SegmentedControlTrigger,
} from "./segmented-control";

function Example() {
  const [value, setValue] = useState("overview");
  return (
    <SegmentedControl value={value} onValueChange={setValue}>
      <SegmentedControlList aria-label="Sections">
        <SegmentedControlTrigger value="overview">
          Overview
        </SegmentedControlTrigger>
        <SegmentedControlTrigger value="access">Access</SegmentedControlTrigger>
      </SegmentedControlList>
      <SegmentedControlContent
        value="overview"
        forceMount
        hidden={value !== "overview"}
      >
        <label>
          Draft
          <input />
        </label>
      </SegmentedControlContent>
      <SegmentedControlContent
        value="access"
        forceMount
        hidden={value !== "access"}
      >
        Permissions
      </SegmentedControlContent>
    </SegmentedControl>
  );
}

it("provides associated tab panels and repeated arrow/Home/End keyboard navigation", async () => {
  const user = userEvent.setup();
  render(<Example />);
  const overview = screen.getByRole("tab", { name: "Overview" });
  const access = screen.getByRole("tab", { name: "Access" });
  expect(screen.getByRole("tabpanel", { name: "Overview" })).toHaveAttribute(
    "id",
    overview.getAttribute("aria-controls"),
  );
  overview.focus();
  await user.keyboard("{ArrowRight}");
  expect(access).toHaveFocus();
  expect(access).toHaveAttribute("aria-selected", "true");
  await user.keyboard("{Home}");
  expect(overview).toHaveFocus();
  await user.keyboard("{End}");
  expect(access).toHaveFocus();
});

it("retains a mounted draft while inactive panels leave accessible navigation", async () => {
  const user = userEvent.setup();
  render(<Example />);
  await user.type(
    screen.getByRole("textbox", { name: "Draft" }),
    "Unsaved name",
  );
  await user.click(screen.getByRole("tab", { name: "Access" }));
  expect(
    screen.queryByRole("textbox", { name: "Draft" }),
  ).not.toBeInTheDocument();
  expect(screen.getByRole("tabpanel", { name: "Access" })).toBeVisible();
  await user.click(screen.getByRole("tab", { name: "Overview" }));
  expect(screen.getByRole("textbox", { name: "Draft" })).toHaveValue(
    "Unsaved name",
  );
});
