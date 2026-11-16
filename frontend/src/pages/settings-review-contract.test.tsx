import "@/i18n";
import type { ComponentProps } from "react";
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { createMemoryHistory, createRootRoute, createRoute, createRouter, RouterProvider, useSearch } from "@tanstack/react-router";
import { TooltipProvider } from "@/components/ui/tooltip";
import { AgentDetailsSheet } from "@/components/assistant/nyxbot-agent-details";
import { SettingsPage } from "./settings";
import { BillingPage } from "./billing";
import { reviewOwner, settingsReviewResponse } from "@/test/settings-review-fixture";
import { useAuthStore } from "@/stores/auth-store";
import { billingFigures } from "@/test/billing-figures";
import baseline from "@/test/billing-figures-baseline.json";
import requestsBaseline from "@/test/settings-requests-baseline.json";
import type { BillingSection } from "@/lib/billing-sections";

// Baselines were captured with this harness and fixture at origin/ios-nested-settings
// (aee1ae9ee9e9858fc9342e519067a5bd8c31f782), before changing presentation.
const captureBase = process.env.SETTINGS_BASELINE_CAPTURE === "1";
let requests: string[];
let client: QueryClient;
const settle = async () => {
  await act(async () => { await new Promise(resolve => setTimeout(resolve, 60)); });
  await waitFor(() => expect(client.isFetching()).toBe(0));
};
beforeEach(() => {
  vi.setSystemTime(new Date("2026-10-15T12:00:00Z"));
  requests = [];
  useAuthStore.setState({ user: reviewOwner });
  client = new QueryClient({ defaultOptions: { queries: { retry: false, staleTime: 60_000 } } });
  const response = (path: string) => {
    requests.push(path);
    return new Response(JSON.stringify(settingsReviewResponse(path)), { headers: { "Content-Type": "application/json" } });
  };
  vi.stubGlobal("fetch", vi.fn(async (input: RequestInfo | URL) => {
    const url = new URL(input instanceof Request ? input.url : String(input), "https://nyxid.test");
    return response(url.pathname.replace(/^\/api\/v1/, "") + url.search);
  }));
  globalThis.__nyxidAssistantHttpMock = ({ endpoint }) => response(endpoint);
});
afterEach(() => { cleanup(); client.clear(); vi.unstubAllGlobals(); vi.useRealTimers(); globalThis.__nyxidAssistantHttpMock = undefined; useAuthStore.setState({ user: null }); });
function AgentRoute() {
  const search = useSearch({ strict: false }) as { details?: ComponentProps<typeof AgentDetailsSheet>["tab"] };
  return <AgentDetailsSheet agentId="agent-researcher" agents={[]} open tab={search.details} onOpenChange={() => {}} onDeleted={() => {}} />;
}
async function open(surface: "account" | "billing" | "agent", section?: string) {
  const root = createRootRoute();
  const pathname = surface === "account" ? "/settings" : surface === "billing" ? "/billing" : "/assistant";
  const route = createRoute({ getParentRoute: () => root, path: pathname, component: () => surface === "account" ? <SettingsPage /> : surface === "billing" ? <BillingPage /> : <AgentRoute />, validateSearch: (search: Record<string, unknown>) => search });
  const router = createRouter({ routeTree: root.addChildren([route]), history: createMemoryHistory({ initialEntries: [pathname + (section ? `?tab=${section}` : "")] }) });
  render(<QueryClientProvider client={client}><TooltipProvider><RouterProvider router={router} /></TooltipProvider></QueryClientProvider>);
  await act(() => router.load()); await settle();
  const go = async (next?: string) => {
    await act(() => router.navigate({ to: pathname, search: surface === "account" ? { tab: next } : { tab: next?.split("/")[0], section: next?.includes("/") ? next as BillingSection : undefined }, replace: true }));
    await settle();
  };
  return go;
}
async function record(surface: string, data: unknown) {
  const { writeFileSync } = await import("node:fs");
  writeFileSync(`/tmp/settings-base-${surface}.json`, JSON.stringify(data, null, 2));
}

it("keeps exact account requests limited to visible summaries and first-visited pages", async () => {
  const go = await open("account");
  const stages: Record<string, string[]> = { root: requests.splice(0).sort() };
  const pages = captureBase ? ["profile", "security", "sessions", "mcp", "display", "privacy"] : ["profile", "security", "security/mfa", "security/password", "security/linked-accounts", "security/delete", "sessions", "mcp", "mcp/cursor", "mcp/claude-code", "mcp/codex", "display", "display/appearance", "display/text", "display/chat", "display/sidebar", "display/colors", "display/colors/surfaces", "display/colors/text", "display/colors/controls", "display/readability", "privacy"];
  for (const section of pages) { await go(section); stages[section] = requests.splice(0).sort(); }
  if (captureBase) await record("account-requests", stages);
  else { expect(stages).toEqual(requestsBaseline.account); }
});

it("keeps billing requests lazy and does not refetch when moving among retained pages", async () => {
  const go = await open("billing");
  const stages: Record<string, string[]> = { root: requests.splice(0).sort() };
  for (const section of captureBase ? ["usage", "billing"] : ["billing/topup", "billing/benefits", "billing/benefits/wallet", "billing/benefits/grants", "billing/benefits/allowances", "billing/bill", "billing/machines", "billing/activity", "usage", "usage/summary", "usage/charts", "usage/charts/quantities", "usage/charts/funding", "usage/explorer"]) {
    await go(section); stages[section] = requests.splice(0).sort();
  }
  if (captureBase) await record("billing-requests", stages);
  else {
    expect(stages).toEqual(requestsBaseline.billing);
    expect(stages.root!.every(path => requestsBaseline.base.billing.root.includes(path))).toBe(true);
    expect(stages.root!.length).toBeLessThanOrEqual(requestsBaseline.base.billing.root.length);
    for (const section of Object.keys(stages).filter(key => key !== "root")) await go(section);
    expect(requests).toEqual([]);
  }
}, 20000);

it("keeps agent detail requests behind their first visit", async () => {
  await open("agent");
  const stages: Record<string, string[]> = { root: requests.splice(0).sort() };
  const user = userEvent.setup({ delay: null });
  const tabs: Record<string, string> = { overview: "Overview", grants: "Access", skills: "Skills", memory: "Memory", channels: "Channels" };
  for (const section of captureBase ? Object.keys(tabs) : ["overview/persona", "overview/access", "overview/chat", "overview/voice", "overview/automations", "overview/ownership", "grants/services", "grants/operations", "grants/guests", "grants/machines", "grants/connections", "skills/installed", "memory", "channels"]) {
    const tab = section.split("/")[0]!;
    if (screen.getByRole("tab", { name: tabs[tab] }).getAttribute("data-state") === "active") await user.click(screen.getByRole("tab", { name: tab === "overview" ? "Skills" : "Overview" }));
    await user.click(screen.getByRole("tab", { name: tabs[tab] }));
    if (!captureBase && section.includes("/")) await user.click(document.querySelector<HTMLElement>(`[data-drawer-page-trigger="${section}"]`)!);
    await settle();
    if (!captureBase && section.includes("/")) expect(document.querySelector(`[data-grouped-detail]:not([hidden])`)).not.toBeNull();
    stages[section] = requests.splice(0).sort();
  }
  if (captureBase) await record("agent-requests", stages);
  else { expect(stages).toEqual(requestsBaseline.agent); }
});

it("preserves every base billing label and figure exactly once across the focused pages", async () => {
  const go = await open("billing");
  const figures: string[] = [];
  const collect = () => figures.push(...billingFigures(document.querySelector<HTMLElement>(".billing-page")!));
  const expand = async () => {
    for (const detail of document.querySelectorAll<HTMLDetailsElement>("details")) {
      if (detail.closest('[hidden],[inert]')) continue;
      detail.open = true; fireEvent(detail, new Event("toggle"));
    }
    await settle();
  };
  const machine = async () => {
    await userEvent.click(screen.getByRole("button", { name: "Review machine runtime" })); await settle();
    await userEvent.click(screen.getByRole("combobox", { name: "Machine runtime billing selection" }));
    await userEvent.click(await screen.findByRole("option", { name: /machine-1/ })); await settle();
  };
  if (captureBase) {
    await userEvent.click(screen.getByRole("button", { name: "View breakdown" }));
    await machine(); await expand(); collect();
    // The old benefits region is shared above both tabs; collect it only once.
    await go("usage"); await expand();
    const benefits = document.querySelector<HTMLElement>(".benefits-container")!; benefits.hidden = true;
    collect(); benefits.hidden = false;
    await record("billing-figures", figures.sort());
  } else {
    collect();
    for (const section of ["billing/topup", "billing/benefits/wallet", "billing/benefits/grants", "billing/benefits/allowances", "billing/bill", "billing/machines", "billing/activity", "usage/summary", "usage/charts/quantities", "usage/charts/funding", "usage/explorer"]) {
      await go(section); if (section === "billing/machines") await machine(); await expand(); collect();
    }
    expect(figures.sort()).toEqual(baseline);
    const summary = document.querySelector(".tab-usage-summary")!;
    const compact = document.querySelector('[data-usage-summary="compact"]')!;
    expect(compact.querySelector("p")!.textContent).toBe(summary.querySelector("strong")!.textContent);
    expect(compact.querySelectorAll("p")[2]!.textContent).toBe(summary.querySelector("dd")!.textContent);
  }
}, 20000);

it("keeps gated agent reads behind the corresponding first visit", async () => {
  await open("agent");
  requests.splice(0);
  act(() => useAuthStore.setState({ user: { ...reviewOwner, capabilities: { ...reviewOwner.capabilities, enabled_features: [
    "assistant:model-controls", "assistant:voice-agent-key", "assistant:agent-learning",
    "assistant:operation-scopes", "assistant:guest-operation-scopes",
  ] } } }));
  await settle();
  expect(requests).toEqual([]);
  const user = userEvent.setup({ delay: null });
  const agentPath = "/assistant/nyxagent/agents/agent-researcher";
  const cases = [
    ["grants/operations", "Access", [`${agentPath}/operations`]],
    ["grants/voice", "Access", [`${agentPath}/voice-service`]],
    ["skills/learning", "Skills", [`${agentPath}/learning`, `${agentPath}/learning/proposals`]],
    ["grants/guests", "Access", [`${agentPath}/guest-readiness`, `${agentPath}/guest-operations`, "/keys"]],
  ] as const;
  for (const [section, tab, expected] of cases) {
    if (screen.getByRole("tab", { name: tab }).getAttribute("data-state") === "active")
      await user.click(screen.getByRole("tab", { name: "Overview" }));
    await user.click(screen.getByRole("tab", { name: tab }));
    await user.click(document.querySelector<HTMLElement>(`[data-drawer-page-trigger="${section}"]`)!);
    await settle();
    expect(requests.splice(0).sort()).toEqual([...expected].sort());
  }
}, 20_000);
