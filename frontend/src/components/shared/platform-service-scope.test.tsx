import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState } from "react";
import { describe, expect, it, vi } from "vitest";
import { PlatformServiceScope } from "./platform-service-scope";

const services = [
  { id: "platform", name: "Search", auto_connected: true },
  { id: "manual", name: "Manual", auto_connected: false },
];
function Harness() {
  const [ids, setIds] = useState(["platform"]);
  const [all, setAll] = useState(false);
  return (
    <PlatformServiceScope
      services={services}
      selectedIds={ids}
      allowAll={all}
      onAllowAllChange={setAll}
      onToggle={(id) =>
        setIds(
          ids.includes(id) ? ids.filter((value) => value !== id) : [...ids, id],
        )
      }
    />
  );
}

describe("PlatformServiceScope", () => {
  it("preserves explicit selection while the durable grant implies disabled checkboxes", async () => {
    const user = userEvent.setup();
    render(<Harness />);
    const service = screen.getByRole("checkbox", { name: "Search" });
    const group = screen.getByRole("checkbox", {
      name: /includes ones added later/,
    });
    expect(service).toBeChecked();
    await user.click(group);
    expect(service).toBeDisabled();
    expect(service).toBeChecked();
    await user.click(group);
    expect(service).toBeEnabled();
    expect(service).toBeChecked();
    await user.click(service);
    expect(service).not.toBeChecked();
    expect(screen.queryByText("Manual")).not.toBeInTheDocument();
  });

  it("explains why a personal platform group is unavailable for an org owner", () => {
    render(
      <PlatformServiceScope
        services={[]}
        selectedIds={[]}
        orgOwned
        onAllowAllChange={vi.fn()}
        onToggle={vi.fn()}
      />,
    );
    expect(screen.queryByRole("checkbox")).not.toBeInTheDocument();
    expect(screen.getByText(/personal account/)).toBeInTheDocument();
  });

  it("offers org-owned platform rows supplied by an owner-filtered picker", () => {
    render(
      <PlatformServiceScope
        services={services}
        selectedIds={[]}
        orgOwned
        onAllowAllChange={vi.fn()}
        onToggle={vi.fn()}
      />,
    );
    expect(screen.getByRole("checkbox", { name: "Search" })).toBeEnabled();
  });
});

it("does not imply another owner's platform service", () => {
  render(
    <PlatformServiceScope
      services={[
        ...services,
        {
          id: "org-platform",
          name: "Org Search",
          auto_connected: true,
          platform_grant_eligible: false,
        },
      ]}
      selectedIds={[]}
      allowAll
      onAllowAllChange={vi.fn()}
      onToggle={vi.fn()}
    />,
  );
  expect(screen.getByRole("checkbox", { name: "Search" })).toBeDisabled();
  const foreign = screen.getByRole("checkbox", { name: /Org Search/ });
  expect(foreign).toBeEnabled();
  expect(foreign).not.toBeChecked();
});
