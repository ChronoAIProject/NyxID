import { useState } from "react";
import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { describe, expect, it, vi } from "vitest";
import { EMPTY_FILTERS } from "@/lib/usage-analytics";
import type { AnalyticsFilters, AnalyticsPanel } from "@/schemas/usage-analytics";
import { FilterBar, PanelControls, type SampleOptions } from "./controls";
import { api } from "@/lib/api-client";
import { appI18n } from "@/i18n";

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

it("translates analytics choices without changing a custom panel title or selected values", async () => {
  const user = userEvent.setup();
  const onChange = vi.fn();
  const panel: AnalyticsPanel = {
    id: "panel", title: "My English report", measure: "cost", metric: "requests",
    chart: "line", breakdown: "service", top: 0, wide: false,
  };
  const view = render(<PanelControls panel={panel} onChange={onChange} />);
  try {
    await act(async () => { await appI18n.changeLanguage("zh-CN"); });
    expect(screen.getByRole("textbox")).toHaveValue("My English report");
    expect(screen.getByRole("combobox", { name: "图表类型" })).toHaveTextContent("折线图");
    expect(screen.getByRole("combobox", { name: "数据表" })).toHaveTextContent("始终展开");
    expect(screen.getByRole("combobox", { name: "显示" })).toHaveTextContent("合计");
    await user.click(screen.getByRole("combobox", { name: "图表类型" }));
    await user.click(screen.getByRole("option", { name: "柱状图" }));
    expect(onChange).toHaveBeenLastCalledWith({ ...panel, chart: "bar" });
    await act(async () => { await appI18n.changeLanguage("en"); });
    expect(screen.getByRole("combobox", { name: "Chart type" })).toHaveTextContent("Line");
    expect(screen.getByRole("textbox")).toHaveValue("My English report");
  } finally {
    view.unmount();
    await appI18n.changeLanguage("en");
  }
});

it("translates time-range choices and still submits the protocol period", async () => {
  const user = userEvent.setup();
  try {
    await act(async () => { await appI18n.changeLanguage("zh-CN"); });
    const onChange = renderBar();
    const range = screen.getByRole("combobox", { name: "时间范围" });
    expect(range).toHaveTextContent("最近 7 天");
    await user.click(range);
    await user.click(screen.getByRole("option", { name: "自定义范围" }));
    expect(onChange).toHaveBeenLastCalledWith(expect.objectContaining({ period: "custom" }));
    await act(async () => { await appI18n.changeLanguage("en"); });
    expect(screen.getByRole("combobox", { name: "Time range" })).toHaveTextContent("Custom range");
  } finally {
    await act(async () => { await appI18n.changeLanguage("en"); });
  }
});

describe("admin usage FilterBar picker", () => {
  it("loads and caches every dropdown before any is opened", async () => {
    const get = vi
      .spyOn(api, "get")
      .mockImplementation(async (path) =>
        path === "/services"
          ? { services: [{ id: "svc-1", name: "Service One", slug: "one" }] }
          : { options: [], total: 0 },
      );
    try {
      render(
        <QueryClientProvider client={new QueryClient()}>
          <FilterBar filters={EMPTY_FILTERS} onChange={vi.fn()} />
        </QueryClientProvider>,
      );
      await waitFor(() => expect(get).toHaveBeenCalledTimes(3));
      expect(get).toHaveBeenCalledWith("/services");
      expect(get).toHaveBeenCalledWith(
        "/admin/usage/identities?kind=actors&search=",
      );
      expect(get).toHaveBeenCalledWith(
        "/admin/usage/identities?kind=owners&search=",
      );
      await userEvent.click(
        screen.getByRole("button", { name: "Filter services" }),
      );
      expect(
        await picker().findByRole("button", { name: /Service One/ }),
      ).toBeVisible();
      expect(get).toHaveBeenCalledTimes(3);
    } finally {
      get.mockRestore();
    }
  });

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
