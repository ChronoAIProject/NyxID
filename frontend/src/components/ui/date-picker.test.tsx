import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState } from "react";
import { describe, expect, it } from "vitest";
import { DatePicker } from "./date-picker";

function MultiDatePickerHarness() {
  const [values, setValues] = useState<readonly string[]>(["2026-07-03"]);
  return (
    <DatePicker
      mode="multiple"
      values={values}
      onValuesChange={setValues}
      ariaLabel="Creation dates"
      placeholder="Select dates"
    />
  );
}

describe("DatePicker", () => {
  it("toggles multiple dates without closing and supports Clear all", async () => {
    const user = userEvent.setup();
    render(<MultiDatePickerHarness />);

    await user.click(screen.getByRole("button", { name: "Creation dates" }));
    const previousMonth = screen.getByRole("button", {
      name: "Previous month",
    });
    const nextMonth = screen.getByRole("button", { name: "Next month" });
    expect(screen.getByText("July 2026")).toBeVisible();

    await user.click(previousMonth);
    expect(screen.getByText("June 2026")).toBeVisible();
    await user.click(nextMonth);
    expect(screen.getByText("July 2026")).toBeVisible();

    const julyThird = screen.getByRole("button", { name: "2026-07-03" });
    const julyEighth = screen.getByRole("button", { name: "2026-07-08" });
    expect(julyThird).toHaveAttribute("aria-pressed", "true");

    await user.click(julyEighth);
    expect(
      screen.getByRole("button", { name: "Creation dates" }),
    ).toHaveTextContent("2 dates selected");
    expect(julyEighth).toHaveAttribute("aria-pressed", "true");

    await user.click(julyThird);
    expect(
      screen.getByRole("button", { name: "Creation dates" }),
    ).toHaveTextContent("July 8, 2026");
    expect(julyThird).toHaveAttribute("aria-pressed", "false");

    await user.click(screen.getByRole("button", { name: "Clear all" }));
    expect(
      screen.getByRole("button", { name: "Creation dates" }),
    ).toHaveTextContent("Select dates");
    expect(screen.getByRole("button", { name: "Done" })).toBeInTheDocument();
  });
});
