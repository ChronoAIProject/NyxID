import {
  cleanup,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import {
  ServiceAvatarStack,
  type ServiceAvatarItem,
} from "./service-avatar-stack";

afterEach(cleanup);

const items: ServiceAvatarItem[] = [
  { id: "personal", type: "personal", name: "Personal" },
  { id: "org", type: "org", name: "ChronoAI development organization" },
  { id: "platform", type: "platform", name: "NyxID platform" },
];

describe("service avatar tooltips", () => {
  it("shows details only for the hovered source without adding an inline label", async () => {
    const user = userEvent.setup();
    render(<ServiceAvatarStack items={items} label="Sources" />);
    const personal = screen.getByRole("button", { name: "Personal · Sources" });
    const org = screen.getByRole("button", {
      name: "ChronoAI development organization · Sources",
    });
    expect(screen.queryByRole("tooltip")).not.toBeInTheDocument();
    await user.hover(personal);
    expect(
      within(await screen.findByRole("tooltip")).getByText("Personal"),
    ).toBeInTheDocument();
    expect(within(personal).queryByText("Personal")).not.toBeInTheDocument();
    await user.hover(org);
    // Happy DOM has no layout; move beyond the tooltip's zero-size grace area.
    await user.pointer({ target: org, coords: { clientX: 40, clientY: 10 } });
    await user.pointer({ target: org, coords: { clientX: 41, clientY: 10 } });
    const tooltip = within(
      await screen.findByRole("tooltip", {
        name: /ChronoAI development organization/,
      }),
    );
    expect(tooltip.queryByText("Personal")).not.toBeInTheDocument();
    expect(
      tooltip.getByText("ChronoAI development organization"),
    ).toBeInTheDocument();
    await user.unhover(org);
    await user.pointer({
      target: document.body,
      coords: { clientX: 100, clientY: 100 },
    });
    await waitFor(() =>
      expect(screen.queryByRole("tooltip")).not.toBeInTheDocument(),
    );
  });

  it("reveals billing on keyboard focus, dismisses with Escape, and opens the selected connection", async () => {
    const user = userEvent.setup();
    const onSelect = vi.fn();
    render(
      <ServiceAvatarStack
        items={[{ ...items[0]!, detail: "Your personal account", onSelect }]}
        label="Billing"
      />,
    );
    await user.tab();
    expect(
      within(await screen.findByRole("tooltip")).getByText(
        "Your personal account",
      ),
    ).toBeInTheDocument();
    await user.keyboard("{Escape}");
    expect(screen.queryByRole("tooltip")).not.toBeInTheDocument();
    await user.keyboard("{Enter}");
    expect(onSelect).toHaveBeenCalledOnce();
  });

  it("keeps sources beyond the initial stack reachable", async () => {
    const user = userEvent.setup();
    const more = Array.from(
      { length: 5 },
      (_, index): ServiceAvatarItem => ({
        id: `org-${index}`,
        type: "org",
        name: `Organization ${index}`,
      }),
    );
    render(<ServiceAvatarStack items={more} label="Sources" />);
    expect(
      screen.queryByRole("button", { name: "Organization 4 · Sources" }),
    ).not.toBeInTheDocument();
    await user.click(
      screen.getByRole("button", { name: "Show all 5 entries · Sources" }),
    );
    const last = screen.getByRole("button", {
      name: "Organization 4 · Sources",
    });
    await user.hover(last);
    expect(
      within(await screen.findByRole("tooltip")).getByText("Organization 4"),
    ).toBeInTheDocument();
    await user.click(
      screen.getByRole("button", { name: "Collapse · Sources" }),
    );
    expect(
      screen.queryByRole("button", { name: "Organization 4 · Sources" }),
    ).not.toBeInTheDocument();
  });
});
