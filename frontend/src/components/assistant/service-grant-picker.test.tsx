import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, expect, it, vi } from "vitest";
import { ServiceGrantPicker } from "./nyxbot-agent-forms";

const platform = vi.hoisted(() => ({ models: [] as { id: string; display_name: string }[] }));
vi.mock("@/hooks/use-platform-models", () => ({ usePlatformModels: () => ({ data: { pages: [{ models: platform.models }] }, hasNextPage: false }) }));
afterEach(() => { platform.models = []; });

vi.mock("@/hooks/use-keys", () => ({
  useKeys: () => ({
    data: [
      {
        id: "first",
        slug: "calendar",
        label: "Calendar one",
        execution_unavailable_reason: "node home-server offline",
        is_active: true,
        credential_source: { type: "org", org_id: "team", allowed: true },
      },
      {
        id: "second",
        slug: "calendar",
        label: "Calendar two",
        is_active: true,
        credential_source: { type: "org", org_id: "team", allowed: true },
      },
      { id: "other", slug: "private", label: "Private", is_active: true },
    ],
  }),
}));

it("uses one personal row per slug and retains unavailable grants for removal", async () => {
  const change = vi.fn();
  render(<ServiceGrantPicker value={["calendar", "gone"]} onChange={change} />);
  expect(screen.getAllByRole("checkbox", { name: /Calendar/ })).toHaveLength(1);
  expect(screen.getByRole("checkbox", { name: /Calendar/ })).toBeChecked();
  await userEvent
    .setup()
    .click(screen.getByRole("checkbox", { name: "gone gone" }));
  expect(change).toHaveBeenCalledWith(["calendar"]);
});

it("keeps separate organization connection IDs even when their slugs match", async () => {
  const change = vi.fn();
  render(<ServiceGrantPicker org="team" value={["first"]} onChange={change} />);
  expect(screen.getAllByRole("checkbox", { name: /Calendar/ })).toHaveLength(2);
  expect(
    screen.queryByRole("checkbox", { name: /Private/ }),
  ).not.toBeInTheDocument();
  await userEvent
    .setup()
    .click(screen.getByRole("checkbox", { name: /Calendar two/ }));
  expect(change).toHaveBeenCalledWith(["first", "second"]);
});

it("keeps an offline service selectable and shows an availability advisory", async () => {
  const change = vi.fn();
  render(<ServiceGrantPicker org="team" value={["first"]} onChange={change} />);
  expect(screen.getByText(/Currently not executable \(node home-server offline\)/)).toBeVisible();
  const row = screen.getByRole("checkbox", { name: /Calendar one/ });
  expect(row).toBeChecked();
  expect(row).toBeEnabled();
  await userEvent.setup().click(row);
  expect(change).toHaveBeenCalledWith([]);
});


it("offers entitled logical models alongside connections using public IDs", async () => {
  platform.models = [{ id: "nyxid/gpt-5", display_name: "GPT-5" }];
  const change = vi.fn();
  render(<ServiceGrantPicker org="team" value={["first"]} onChange={change} />);
  await userEvent.setup().click(screen.getByRole("checkbox", { name: /GPT-5/ }));
  expect(change).toHaveBeenCalledWith(["first", "nyxid/gpt-5"]);
  expect(screen.getAllByRole("checkbox", { name: /Calendar/ })).toHaveLength(2);
});
