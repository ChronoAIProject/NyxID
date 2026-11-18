import "@/i18n";
import type { ComponentProps } from "react";
import { act, cleanup, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, expect, it, onTestFinished, vi } from "vitest";
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
import { settingsReviewEnvironment, settleSettingsReviewQueries } from "@/test/settings-review-environment";

// Base renders come from aee1ae9ee9e9858fc9342e519067a5bd8c31f782,
// with the same fixture, en-US/UTC native formatting and fixed clock as this file.
const captureBase = process.env.SETTINGS_BASELINE_CAPTURE === "1";
// Walk every retained page with shared CI runner headroom.
const CONTRACT_TIMEOUT = 30_000;
beforeEach(settingsReviewEnvironment);
afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
  vi.useRealTimers();
  globalThis.__nyxidAssistantHttpMock = undefined;
  useAuthStore.setState({ user: null });
});
function AgentRoute() {
  const search = useSearch({ strict: false }) as { details?: ComponentProps<typeof AgentDetailsSheet>["tab"] };
  return <AgentDetailsSheet agentId="agent-researcher" agents={[]} open tab={search.details} onOpenChange={() => {}} onDeleted={() => {}} />;
}
async function open(surface: "account" | "billing" | "agent", section?: string) {
  const requests: string[] = [];
  const client = new QueryClient({ defaultOptions: { queries: { retry: false, staleTime: Infinity } } });
  useAuthStore.setState({ user: structuredClone(reviewOwner) });
  const response = (path: string) => {
    requests.push(path);
    return new Response(JSON.stringify(settingsReviewResponse(path)), { headers: { "Content-Type": "application/json" } });
  };
  vi.stubGlobal("fetch", vi.fn(async (input: RequestInfo | URL) => {
    const url = new URL(input instanceof Request ? input.url : String(input), "https://nyxid.test");
    return response(url.pathname.replace(/^\/api\/v1/, "") + url.search);
  }));
  globalThis.__nyxidAssistantHttpMock = ({ endpoint }) => response(endpoint);
  const settle = () => settleSettingsReviewQueries(client);
  const root = createRootRoute();
  const pathname = surface === "account" ? "/settings" : surface === "billing" ? "/billing" : "/assistant";
  const route = createRoute({ getParentRoute: () => root, path: pathname, component: () => surface === "account" ? <SettingsPage /> : surface === "billing" ? <BillingPage /> : <AgentRoute />, validateSearch: (search: Record<string, unknown>) => search });
  const router = createRouter({ routeTree: root.addChildren([route]), history: createMemoryHistory({ initialEntries: [pathname + (section ? `?tab=${section}` : "")] }) });
  render(<QueryClientProvider client={client}><TooltipProvider><RouterProvider router={router} /></TooltipProvider></QueryClientProvider>);
  onTestFinished(async () => {
    await client.cancelQueries();
    client.clear();
  });
  await act(() => router.load());
  if (surface === "agent") await screen.findByRole("tab", { name: "Overview" });
  else if (surface === "account") await screen.findByRole("region", { name: "Account settings" });
  else await screen.findByRole("tab", { name: "Billing" });
  await settle();
  const go = async (next?: string) => {
    await act(() => router.navigate({ to: pathname, search: surface === "agent" ? { details: next as ComponentProps<typeof AgentDetailsSheet>["tab"] } : surface === "account" ? { tab: next } : { tab: next?.split("/")[0], section: next?.includes("/") ? next as BillingSection : undefined }, replace: true }));
    await settle();
  };
  return { go, requests, settle, user: userEvent.setup({ delay: null }) };
}
async function record(surface: string, data: unknown) {
  const { writeFileSync } = await import("node:fs");
  writeFileSync(`/tmp/settings-base-${surface}.json`, JSON.stringify(data, null, 2));
}

it("keeps exact account requests limited to visible summaries and first-visited pages", async () => {
  const { go, requests } = await open("account");
  const stages: Record<string, string[]> = { root: requests.splice(0).sort() };
  const pages = captureBase ? ["profile", "security", "sessions", "mcp", "display", "privacy"] : ["profile", "security", "security/delete", "sessions", "mcp", "display", "display/layout", "display/colors", "privacy"];
  for (const section of pages) { await go(section); stages[section] = requests.splice(0).sort(); }
  if (captureBase) await record("account-requests", stages);
  else { expect(stages).toEqual(requestsBaseline.account); }
}, CONTRACT_TIMEOUT);

it("keeps wide account requests behind their first category visit, with Profile visible on open", async () => {
  const original = HTMLElement.prototype.getBoundingClientRect;
  vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (this: HTMLElement) {
    return this.parentElement?.hasAttribute("data-form-viewport") ? { ...original.call(this), width: 800 } : original.call(this);
  });
  const { go, requests } = await open("account");
  expect(screen.getByRole("textbox", { name: "Name" })).toBeVisible();
  const stages: Record<string, string[]> = { root: requests.splice(0).sort() };
  for (const section of ["profile", "security", "security/delete", "sessions", "mcp", "display", "display/layout", "display/colors", "privacy"]) {
    await go(section); stages[section] = requests.splice(0).sort();
  }
  expect(stages).toEqual({ ...requestsBaseline.account, root: ["/users/me"], sessions: ["/sessions"] });
  for (const section of Object.keys(stages).filter(section => section !== "root")) await go(section);
  expect(requests).toEqual([]);
}, CONTRACT_TIMEOUT);

it("keeps billing requests lazy and does not refetch when moving among retained pages", async () => {
  const { go, requests } = await open("billing");
  const stages: Record<string, string[]> = { root: requests.splice(0).sort() };
  for (const section of captureBase ? ["usage", "billing"] : ["billing/topup", "billing/benefits", "billing/bill", "billing/machines", "billing/activity", "usage", "usage/charts", "usage/explorer"]) {
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
}, CONTRACT_TIMEOUT);

it("keeps agent detail requests behind their first visit", async () => {
  const { go, requests } = await open("agent");
  const stages: Record<string, string[]> = { root: requests.splice(0).sort() };
  const tabs: Record<string, string> = { overview: "Overview", grants: "Access", skills: "Skills", memory: "Memory", channels: "Channels" };
  for (const section of captureBase ? Object.keys(tabs) : ["overview/persona", "overview/voice", "overview/automations", "grants", "grants/services", "grants/machines", "grants/connections", "skills", "memory", "channels"]) {
    await go(section);
    if (!captureBase && section.includes("/"))
      expect(document.querySelector(`[data-grouped-section="${section}"]:not([hidden])`)).not.toBeNull();
    stages[section] = requests.splice(0).sort();
  }
  if (captureBase) await record("agent-requests", stages);
  else { expect(stages).toEqual(requestsBaseline.agent); }
}, CONTRACT_TIMEOUT);

it("preserves every base billing label and figure exactly once across the focused pages", async () => {
  const { go, settle, user } = await open("billing");
  const figures: string[] = [];
  const collect = () => figures.push(...billingFigures(document.querySelector<HTMLElement>(".billing-page")!));
  const expand = async () => {
    for (const button of screen.queryAllByRole("button", { name: "Show all" })) await user.click(button);
    for (const detail of document.querySelectorAll<HTMLDetailsElement>("details")) {
      if (detail.closest('[hidden],[inert]') || detail.open) continue;
      await user.click(detail.querySelector("summary")!);
    }
    await settle();
  };
  const machine = async () => {
    await user.click(screen.getByRole("button", { name: "Review machine runtime" })); await settle();
    await user.click(screen.getByRole("combobox", { name: "Machine runtime billing selection" }));
    await user.click(await screen.findByRole("option", { name: /machine-1/ })); await settle();
  };
  if (captureBase) {
    await user.click(screen.getByRole("button", { name: "View breakdown" }));
    await machine(); await expand(); collect();
    // The old benefits region is shared above both tabs; collect it only once.
    await go("usage"); await expand();
    const benefits = document.querySelector<HTMLElement>(".benefits-container")!; benefits.hidden = true;
    collect(); benefits.hidden = false;
    await record("billing-figures", figures.sort());
  } else {
    collect();
    const compact = document.querySelector('[data-usage-summary="compact"]')!.cloneNode(true) as HTMLElement;
    for (const section of ["billing/topup", "billing/benefits", "billing/bill", "billing/machines", "billing/activity", "usage", "usage/charts", "usage/explorer"]) {
      await go(section); if (section === "billing/machines") await machine(); await expand();
      const benefits = document.querySelector<HTMLElement>(".benefits-container");
      if (section === "usage" && benefits) benefits.hidden = true;
      collect();
      if (benefits) benefits.hidden = false;
    }
    expect(figures.sort()).toEqual(baseline);
    const summary = document.querySelector(".tab-usage-summary")!;
    expect(compact.querySelector("p")!.textContent).toBe(summary.querySelector("strong")!.textContent);
    expect(compact.querySelectorAll("p")[2]!.textContent).toBe(summary.querySelector("dd")!.textContent);
  }
}, CONTRACT_TIMEOUT);

it("keeps gated agent reads behind the corresponding first visit", async () => {
  const { go, requests, settle } = await open("agent");
  requests.splice(0);
  act(() => useAuthStore.setState({ user: { ...reviewOwner, capabilities: { ...reviewOwner.capabilities, enabled_features: [
    "assistant:model-controls", "assistant:voice-agent-key", "assistant:agent-learning",
    "assistant:operation-scopes", "assistant:guest-operation-scopes",
  ] } } }));
  await settle();
  expect(requests).toEqual([]);
  const agentPath = "/assistant/nyxagent/agents/agent-researcher";
  const cases = [
    ["grants", [`${agentPath}/voice-service`]],
    ["grants/services", [`${agentPath}/operations`, `${agentPath}/guest-readiness`, `${agentPath}/guest-operations`, "/assistant/nyxagent/grant-services", "/keys"]],
    ["skills", [`${agentPath}/learning`, `${agentPath}/learning/proposals`, `${agentPath}/skills`, `${agentPath}/skills/history`]],
  ] as const;
  for (const [section, expected] of cases) {
    await go(section);
    expect(requests.splice(0).sort()).toEqual([...expected].sort());
  }
}, CONTRACT_TIMEOUT);
