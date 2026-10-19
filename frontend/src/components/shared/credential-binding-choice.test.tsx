import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { expect, it, vi } from "vitest";
import { CredentialBindingChoice } from "./credential-binding-choice";
it("shows both lane prices and allows choosing your own key", async () => {
  const onChange = vi.fn();
  render(
    <CredentialBindingChoice
      value
      onChange={onChange}
      platformPrice={{
        metric: "tokens",
        credits_per_unit: "0.25",
        sync_status: "synced",
      }}
    />,
  );
  expect(screen.getByRole("radio", { name: /Use NyxID's key/ })).toBeChecked();
  expect(screen.getByText("0.25 credits / token")).toBeInTheDocument();
  await userEvent.click(
    screen.getByRole("radio", { name: /Use your own key/ }),
  );
  expect(onChange).toHaveBeenCalledWith(false);
});
