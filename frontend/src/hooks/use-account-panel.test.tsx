import { act, cleanup, render } from "@testing-library/react";
import { createMemoryHistory, createRootRoute, createRoute, createRouter, RouterProvider } from "@tanstack/react-router";
import { afterEach, expect, it, vi } from "vitest";
import { withAccountPanelSearch } from "@/lib/assistant/account-panel-search";
import { useAccountPanel } from "./use-account-panel";

afterEach(() => { cleanup(); vi.useRealTimers(); });

it("rejects old panel writers after close and reopen on the same route", async () => {
  let account: ReturnType<typeof useAccountPanel> | undefined;
  const root = createRootRoute();
  const route = createRoute({
    getParentRoute: () => root,
    path: "/assistant",
    validateSearch: withAccountPanelSearch(() => ({})),
    component: function PanelProbe() { account = useAccountPanel(); return null; },
  });
  const router = createRouter({ routeTree: root.addChildren([route]), history: createMemoryHistory({ initialEntries: ["/assistant?panel=settings"] }) });
  render(<RouterProvider router={router} />);
  await act(() => router.load());
  const staleUpdate = account!.update;
  await act(() => account!.close());
  await act(() => account!.open("settings"));
  await act(() => staleUpdate({ panelTab: "security" }));
  expect(router.state.location.search).toEqual({ panel: "settings" });
  await act(() => account!.update({ panelTab: "privacy" }));
  expect(router.state.location.search).toEqual({ panel: "settings", panelTab: "privacy" });
});

it("cancels a deferred menu selection after leaving and returning to the route", async () => {
  let account: ReturnType<typeof useAccountPanel> | undefined;
  const root = createRootRoute();
  const route = createRoute({ getParentRoute: () => root, path: "/assistant", validateSearch: withAccountPanelSearch(() => ({})), component: function PanelProbe() { account = useAccountPanel(); return null; } });
  const other = createRoute({ getParentRoute: () => root, path: "/assistant/plugins", component: () => null });
  const router = createRouter({ routeTree: root.addChildren([route, other]), history: createMemoryHistory({ initialEntries: ["/assistant"] }) });
  render(<RouterProvider router={router} />);
  await act(() => router.load());
  const dismiss = vi.fn();
  const request = account!.prepareMenuOpen("settings", {}, null, dismiss);
  await act(() => router.navigate({ to: "/assistant/plugins" }));
  await act(() => router.navigate({ to: "/assistant" }));
  vi.useFakeTimers();
  request();
  await act(() => vi.runAllTimersAsync());
  expect(dismiss).not.toHaveBeenCalled();
  expect(router.state.location.search).toEqual({});
});

it("pushes opening NyxBot and replaces every drill-in and parent so browser Back closes it", async () => {
  let account: ReturnType<typeof useAccountPanel> | undefined;
  const root = createRootRoute();
  const route = createRoute({ getParentRoute: () => root, path: "/assistant", validateSearch: withAccountPanelSearch(() => ({})), component: function Probe() { account = useAccountPanel(); return null; } });
  const history = createMemoryHistory({ initialEntries: ["/assistant"] });
  const router = createRouter({ routeTree: root.addChildren([route]), history });
  render(<RouterProvider router={router} />);
  await act(() => router.load());
  await act(() => account!.open("nyxbot"));
  expect(history.length).toBe(2);
  for (const panelTab of ["tasks", "tasks/specialists", "tasks", undefined]) {
    await act(() => account!.update({ panelTab }));
    expect(history.length).toBe(2);
    expect(router.state.location.search).toEqual({ panel: "nyxbot", ...(panelTab ? { panelTab } : {}) });
  }
  await act(() => history.back());
  expect(router.state.location.search).toEqual({});
});

it("allows a caller to force canonical URL replacement without naming a panel", async () => {
  let account: ReturnType<typeof useAccountPanel> | undefined;
  const root = createRootRoute();
  const route = createRoute({ getParentRoute: () => root, path: "/assistant", validateSearch: withAccountPanelSearch(() => ({})), component: function Probe() { account = useAccountPanel(); return null; } });
  const history = createMemoryHistory({ initialEntries: ["/assistant?panel=settings&panelTab=profile"] });
  const router = createRouter({ routeTree: root.addChildren([route]), history });
  render(<RouterProvider router={router} />);
  await act(() => router.load());
  const navigate = vi.spyOn(router, "navigate");
  await act(() => account!.update({ panelTab: "profile" }));
  expect(navigate).not.toHaveBeenCalled();
  await act(() => account!.update({ panelTab: "profile" }, "replace", { force: true }));
  expect(navigate).toHaveBeenCalledOnce();
  expect(navigate).toHaveBeenCalledWith(expect.objectContaining({ replace: true, resetScroll: false }));
  expect(router.state.location.searchStr).toContain("profile");
  expect(history.length).toBe(1);
});
