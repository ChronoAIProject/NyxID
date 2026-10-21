import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, it, vi } from "vitest";
import { NyxbotModelUsage } from "./nyxbot-model-usage";
import { modelCost, modelDailyPoints } from "@/lib/nyxbot-model-usage";
import { ModelTrends } from "./nyxbot-model-trends";
import { useNyxbotModelUsage } from "@/hooks/use-nyxbot-model-usage";
import { nyxbotModelUsageSchema } from "@/schemas/nyxbot-model-usage";
import { modelStats, nyxbotModelFixture } from "@/test/nyxbot-model-fixture";
vi.mock("@/hooks/use-nyxbot-model-usage", () => ({
  useNyxbotModelUsage: vi.fn(),
}));
const refetch = vi.fn();
function setQuery(extra = {}) {
  vi.mocked(useNyxbotModelUsage).mockReturnValue({
    data: nyxbotModelFixture(),
    isPending: false,
    isError: false,
    isFetching: false,
    refetch,
    ...extra,
  } as unknown as ReturnType<typeof useNyxbotModelUsage>);
}
beforeEach(() => {
  vi.clearAllMocks();
  setQuery();
});
it("renders precise billing and traffic, keeps model drilldown keyboard accessible", async () => {
  render(
    <NyxbotModelUsage from="2026-10-10T00:00:00Z" to="2026-10-17T00:00:00Z" />,
  );
  expect(
    screen.getByRole("heading", { name: "Gross credits · Daily UTC" }),
  ).toBeInTheDocument();
  expect(
    screen.getByRole("heading", { name: "Requests · Daily UTC" }),
  ).toBeInTheDocument();
  expect(
    screen.getByRole("heading", { name: "Total tokens · Daily UTC" }),
  ).toBeInTheDocument();
  expect(screen.getAllByText("0.000000000001").length).toBeGreaterThan(0);
  screen.getAllByRole("button", { name: "gpt-5.4" })[0]!.focus();
  await userEvent.keyboard("{Enter}");
  expect(useNyxbotModelUsage).toHaveBeenLastCalledWith(
    expect.any(String),
    expect.any(String),
    { model: "gpt-5.4" },
    true,
  );
  await userEvent.click(screen.getByRole("button", { name: "All models" }));
  expect(useNyxbotModelUsage).toHaveBeenLastCalledWith(
    expect.any(String),
    expect.any(String),
    {},
    true,
  );
  expect(
    modelCost(
      modelStats({ gross_cost: null, unknown_cost_events: 2 }),
      "gross_cost",
    ),
  ).toBe("Unavailable");
  expect(
    modelCost(
      modelStats({ gross_cost: "0.000000000001", unknown_cost_events: 2 }),
      "gross_cost",
    ),
  ).toBe("≥ 0.000000000001");
});
it("keeps fixed model series with zeros for absent days, null for unknown and exact tooltip strings", () => {
  const data = nyxbotModelFixture();
  data.daily = data.daily.filter((d) => d.day.startsWith("2026-10-10"));
  const points = modelDailyPoints(data, "gross_cost");
  expect(points).toHaveLength(7);
  expect(points[0]!.s2).toBeNull();
  expect(points[0]!.exact1).toBe("0.000000000001");
  expect(points[1]!.s0).toBe(0);
});
it("falls back when a filtered model no longer reports the selected audio metric", async () => {
  const data = nyxbotModelFixture();
  const { rerender } = render(<ModelTrends data={data} kind="traffic" />);
  await userEvent.click(
    screen.getByRole("combobox", { name: "Token measure" }),
  );
  await userEvent.click(
    screen.getByRole("option", { name: "Audio input tokens" }),
  );
  expect(
    screen.getByRole("heading", { name: "Audio input tokens · Daily UTC" }),
  ).toBeInTheDocument();
  rerender(
    <ModelTrends
      data={nyxbotModelFixture(undefined, undefined, "claude-sonnet-4-5")}
      kind="traffic"
    />,
  );
  expect(
    screen.getByRole("heading", { name: "Total tokens · Daily UTC" }),
  ).toBeInTheDocument();
});
it("shows loading, retry and empty states", async () => {
  setQuery({ data: undefined, isPending: true });
  const { rerender } = render(<NyxbotModelUsage from="a" to="b" />);
  expect(
    screen.getByRole("status", { name: "Loading model usage" }),
  ).toBeInTheDocument();
  setQuery({ data: undefined, isError: true });
  rerender(<NyxbotModelUsage from="a" to="b" />);
  await userEvent.click(screen.getByRole("button", { name: /retry/i }));
  expect(refetch).toHaveBeenCalled();
  const data = nyxbotModelFixture();
  data.totals.events = 0;
  setQuery({ data });
  rerender(<NyxbotModelUsage from="a" to="b" />);
  expect(screen.getByText(/No attributed model usage/)).toBeInTheDocument();
});
it("shows top billing accounts as the dashboard panel with model detail on demand", async () => {
  const data = nyxbotModelFixture();
  data.other_accounts = {
    model: null,
    is_other: true,
    groups: 4,
    usage: modelStats({ gross_cost: "1.5", requests: 2 }),
  };
  setQuery({ data });
  render(
    <NyxbotModelUsage from="2026-10-10T00:00:00Z" to="2026-10-17T00:00:00Z" />,
  );
  const panel = screen.getByRole("article", { name: "Top billing accounts" });
  expect(
    within(panel).getByText("Gross cost · Billing account"),
  ).toBeInTheDocument();
  expect(within(panel).getByText("Top 10 + Other")).toBeInTheDocument();
  const table = within(panel).getByRole("table", {
    name: "Top billing accounts data",
  });
  expect(within(table).getAllByRole("row")).toHaveLength(
    data.accounts.length + 2,
  );
  for (const { account } of data.accounts) {
    expect(
      within(table).getByRole("cell", { name: account.display_name }),
    ).toBeInTheDocument();
  }
  expect(
    within(table).getByRole("cell", { name: "Other" }),
  ).toBeInTheDocument();
  const detail = within(panel).getByText("Models by billing account");
  expect(
    within(panel).getByRole("table", {
      name: "Alice Research usage by model",
      hidden: true,
    }),
  ).not.toBeVisible();
  await userEvent.click(detail);
  expect(
    within(panel).getByRole("table", { name: "Alice Research usage by model" }),
  ).toBeVisible();
  expect(
    within(panel).getByText("Other · 4 billing accounts"),
  ).toBeInTheDocument();
});
it("breaks model executions down by inference service, including unknown models", async () => {
  const data = nyxbotModelFixture();
  data.services.push({
    service: null,
    is_other: true,
    groups: 3,
    usage: modelStats({ requests: 5, gross_cost: null }),
  });
  setQuery({ data });
  render(
    <NyxbotModelUsage from="2026-10-10T00:00:00Z" to="2026-10-17T00:00:00Z" />,
  );
  const panel = screen.getByRole("article", { name: "Model services" });
  expect(within(panel).getByText("Requests · Service")).toBeInTheDocument();
  expect(within(panel).getByText("Top 20 + Other")).toBeInTheDocument();
  const table = within(panel).getByRole("table", {
    name: "Model services data",
  });
  for (const label of ["Chrono LLM (public)", "Anthropic", "Other"]) {
    expect(
      within(table).getByRole("cell", { name: label }),
    ).toBeInTheDocument();
  }
  // Request counts are exact and never carry cost caveats.
  expect(
    within(panel).queryByText(/metering events have unknown cost/),
  ).not.toBeInTheDocument();
  await userEvent.click(within(panel).getByText("Cost and tokens by service"));
  const detail = within(panel).getByRole("table", {
    name: "Model services usage",
  });
  expect(
    within(detail).getByRole("rowheader", { name: "Other · 3 services" }),
  ).toBeVisible();
  expect(
    within(detail).getByRole("rowheader", { name: "Chrono LLM (public)" }),
  ).toBeVisible();
});
it("accepts reports from servers without the service breakdown", () => {
  const legacy: Record<string, unknown> = { ...nyxbotModelFixture() };
  delete legacy.services;
  expect(nyxbotModelUsageSchema.parse(legacy).services).toEqual([]);
});
