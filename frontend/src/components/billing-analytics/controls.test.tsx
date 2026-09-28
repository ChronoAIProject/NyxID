import { useState } from "react";
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { describe, expect, it, vi } from "vitest";
import { EMPTY_FILTERS } from "@/lib/usage-analytics";
import type { AnalyticsFilters } from "@/schemas/usage-analytics";
import { FilterBar, type SampleOptions } from "./controls";

const services = Array.from({ length: 22 }, (_, index) => ({
  id: `svc-${index}`,
  label: `Service ${index}`,
  detail: `service-${index}`,
}));
const sample: SampleOptions = { services, actors: [], owners: [] };

function renderBar(initial: Partial<AnalyticsFilters> = {}) {
  const onChange = vi.fn();
  function Harness() {
    const [filters, setFilters] = useState<AnalyticsFilters>({
      ...EMPTY_FILTERS,
      ...initial,
    });
    return (
      <FilterBar
        filters={filters}
        sample={sample}
        onChange={(next) => {
          onChange(next);
          setFilters(next);
        }}
      />
    );
  }
  render(
    <QueryClientProvider client={new QueryClient()}>
      <Harness />
    </QueryClientProvider>,
  );
  return onChange;
}

const picker = () => within(screen.getByRole("dialog"));

describe("admin usage FilterBar picker", () => {
  it("matches a selected slug to its service and keeps it through Apply", async () => {
    const onChange = renderBar({ services: ["service-3"] });
    await userEvent.click(
      screen.getByRole("button", { name: "Filter services" }),
    );
    expect(
      await picker().findByRole("button", { name: /Service 3\b/ }),
    ).toHaveAttribute("aria-pressed", "true");
    await userEvent.click(
      picker().getByRole("button", { name: /Service 4\b/ }),
    );
    await userEvent.click(picker().getByRole("button", { name: "Apply" }));
    expect(onChange).toHaveBeenLastCalledWith(
      expect.objectContaining({ services: ["service-3", "svc-4"] }),
    );
  });

  it("preserves selections hidden by the current search", async () => {
    const onChange = renderBar({ services: ["svc-1"] });
    await userEvent.click(
      screen.getByRole("button", { name: "Filter services" }),
    );
    await userEvent.type(
      screen.getByRole("textbox", { name: "Search services" }),
      "Service 2",
    );
    await userEvent.click(
      await picker().findByRole("button", { name: /Service 2\b/ }),
    );
    expect(
      picker().queryByRole("button", { name: /Service 1\b/ }),
    ).not.toBeInTheDocument();
    await userEvent.click(picker().getByRole("button", { name: "Apply" }));
    expect(onChange).toHaveBeenLastCalledWith(
      expect.objectContaining({ services: ["svc-1", "svc-2"] }),
    );
  });

  it("discards the draft on Cancel", async () => {
    const onChange = renderBar();
    await userEvent.click(
      screen.getByRole("button", { name: "Filter services" }),
    );
    await userEvent.click(
      await picker().findByRole("button", { name: /Service 0\b/ }),
    );
    await userEvent.click(picker().getByRole("button", { name: "Cancel" }));
    expect(onChange).not.toHaveBeenCalled();
  });

  it("caps a selection at 20 services", async () => {
    renderBar({ services: services.slice(0, 20).map((option) => option.id) });
    await userEvent.click(
      screen.getByRole("button", { name: "Filter services" }),
    );
    expect(await screen.findByText("20 selected")).toBeVisible();
    expect(
      picker().getByRole("button", { name: /Service 21\b/ }),
    ).toBeDisabled();
    expect(picker().getByRole("button", { name: /Service 0\b/ })).toBeEnabled();
  });
});
