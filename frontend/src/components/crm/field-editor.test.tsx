import { useState } from "react";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { expect, it } from "vitest";
import { CrmFieldEditor } from "./field-editor";

it("keeps optional boolean unset, false and true distinct and allows clearing", async () => {
  function Editor() {
    const [value, setValue] = useState<unknown>(null);
    return (
      <>
        <CrmFieldEditor
          field={{
            key: "active",
            label: "Active",
            type: "boolean",
            required: false,
            archived: false,
          }}
          value={value}
          onChange={setValue}
        />
        <output aria-label="Stored value">{JSON.stringify(value)}</output>
      </>
    );
  }
  render(<Editor />);
  const user = userEvent.setup();
  expect(screen.getByRole("combobox", { name: "Active" })).toHaveTextContent(
    "Not set",
  );
  await user.click(screen.getByRole("combobox", { name: "Active" }));
  await user.click(screen.getByRole("option", { name: "No" }));
  expect(screen.getByLabelText("Stored value")).toHaveTextContent("false");
  await user.click(screen.getByRole("combobox", { name: "Active" }));
  await user.click(screen.getByRole("option", { name: "Yes" }));
  expect(screen.getByLabelText("Stored value")).toHaveTextContent("true");
  await user.click(screen.getByRole("combobox", { name: "Active" }));
  await user.click(screen.getByRole("option", { name: "Not set" }));
  expect(screen.getByLabelText("Stored value")).toHaveTextContent("null");
});
