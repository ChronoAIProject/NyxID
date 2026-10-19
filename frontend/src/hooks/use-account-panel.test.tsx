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
