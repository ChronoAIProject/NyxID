import { useState } from "react";
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { expect, it, vi } from "vitest";
import { OrgScopeSelect } from "./org-scope-select";

vi.mock("@/hooks/use-orgs", () => ({
  useOrgs: () => ({
    isLoading: false,
    data: [
      { id: "admin-org", display_name: "Support", your_role: "admin" },
      { id: "member-org", display_name: "Members", your_role: "member" },
    ],
  }),
}));

function Picker({ allowAll = false }: { readonly allowAll?: boolean }) {
  const [scope, setScope] = useState<string | null>(allowAll ? "all" : null);
  return <OrgScopeSelect value={scope} onChange={setScope} allowAll={allowAll} personalLabel={allowAll ? "User" : undefined} />;
}

it("places View all above a divider and offers only administered organizations", async () => {
  const user = userEvent.setup();
  render(<Picker allowAll />);
  await user.click(screen.getByRole("combobox", { name: "Scope" }));
  const list = within(screen.getByRole("listbox"));
  const options = list.getAllByRole("option");
  expect(options.map((option) => option.textContent)).toEqual(["View all", "User", "Support"]);
  const separator = list.getByRole("separator", { hidden: true });
  expect(options[0]!.compareDocumentPosition(separator) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  expect(separator.compareDocumentPosition(options[1]!) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  await user.click(list.getByRole("option", { name: "Support" }));
  expect(screen.getByRole("combobox")).toHaveTextContent("Support");
});

it("keeps aggregate scope out of creation pickers", async () => {
  const user = userEvent.setup();
  render(<Picker />);
  await user.click(screen.getByRole("combobox", { name: "Scope" }));
  expect(screen.queryByRole("option", { name: "View all" })).not.toBeInTheDocument();
  expect(screen.queryByRole("separator", { hidden: true })).not.toBeInTheDocument();
});
