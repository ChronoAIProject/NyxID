import { useCallback, useMemo } from "react";
import { useNavigate, useRouter, useRouterState } from "@tanstack/react-router";
import {
  isAssistantShellRoute, parseAccountPanelSearch, withoutAccountPanel,
  type AccountPanel, type AccountPanelParams,
} from "@/lib/assistant/account-panel-search";
import { useAuthStore } from "@/stores/auth-store";

interface PanelCoordinator {
  writes: Promise<void>;
  opener: HTMLElement | null;
  generation: number;
}
const coordinators = new WeakMap<object, PanelCoordinator>();
function coordinatorFor(router: ReturnType<typeof useRouter>) {
  let value = coordinators.get(router);
  if (!value) {
    value = { writes: Promise.resolve(), opener: null, generation: 0 };
    const coordinator = value;
    router.subscribe("onBeforeNavigate", ({ fromLocation, toLocation }) => {
      if (fromLocation?.pathname !== toLocation.pathname ||
        parseAccountPanelSearch(fromLocation?.search ?? {}).panel !== parseAccountPanelSearch(toLocation.search).panel) {
        coordinator.generation++;
      }
    });
    coordinators.set(router, value);
  }
  return value;
}

export function useAccountPanel() {
  const router = useRouter();
  const navigate = useNavigate();
  const location = useRouterState({ select: (state) => state.location });
  const current = parseAccountPanelSearch(location.search);
  const coordinator = coordinatorFor(router);
  const panelGeneration = coordinator.generation;

  const enqueue = useCallback((work: () => Promise<void>) => {
    const result = coordinator.writes.then(work);
    coordinator.writes = result.catch(() => undefined);
    return result;
  }, [coordinator]);

  const change = useCallback((
    mode: "open" | "update" | "close",
    params: AccountPanelParams = {},
    options: { history?: "push" | "replace"; panel?: AccountPanel; opener?: HTMLElement | null } = {},
  ) => {
    const pathname = location.pathname;
    const expectedPanel = current.panel;
    const actor = useAuthStore.getState().user?.id;
    const generation = panelGeneration;
    return enqueue(async () => {
      if (!isAssistantShellRoute(pathname) || router.state.location.pathname !== pathname ||
        useAuthStore.getState().user?.id !== actor || coordinator.generation !== generation) return;
      const live = parseAccountPanelSearch(router.state.location.search);
      if (mode !== "open" && (!expectedPanel || live.panel !== expectedPanel || coordinator.generation !== generation)) return;
      const replace = options.history === "replace";
      if (mode === "update" && JSON.stringify(live) === JSON.stringify(parseAccountPanelSearch({
        ...live, ...(!replace ? { panelAction: undefined } : {}), ...params,
      }))) return;
      // Consume an abandoned top-up in its own history entry before pushing.
      if (!replace && live.panelAction) {
        await navigate({ from: pathname, to: pathname, search: (previous) => ({
          ...previous, panelAction: undefined,
        }), replace: true, resetScroll: false });
      }
      if (router.state.location.pathname !== pathname || useAuthStore.getState().user?.id !== actor || coordinator.generation !== generation) return;
      if (mode === "open") coordinator.opener = options.opener ?? null;
      await navigate({ from: pathname, to: pathname, search: (previous) => {
        const fresh = parseAccountPanelSearch(previous);
        if (mode !== "open" && (fresh.panel !== expectedPanel || coordinator.generation !== generation)) return previous;
        const base = withoutAccountPanel(previous);
        if (mode === "close") return base;
        return {
          ...base,
          ...parseAccountPanelSearch({
            ...(mode === "update" ? fresh : { panel: options.panel }),
            ...(!replace ? { panelAction: undefined } : {}),
            ...params,
          }),
        };
      }, replace, resetScroll: false });
    });
  }, [location.pathname, current.panel, enqueue, router, navigate, coordinator, panelGeneration]);

  const close = useCallback((history: "push" | "replace" = "push") => change("close", {}, { history }), [change]);
  return useMemo(() => ({
    current,
    open: (panel: AccountPanel, params: AccountPanelParams = {}, opener: HTMLElement | null =
      document.activeElement instanceof HTMLElement ? document.activeElement : null) =>
      change("open", params, { panel, opener }),
    update: (params: AccountPanelParams, history: "push" | "replace" = "replace") =>
      change("update", params, { history }),
    close,
    opener: () => coordinator.opener,
    // Schedule outside the menu's lifetime: drawer dismissal unmounts that menu.
    prepareMenuOpen: (panel: AccountPanel, params: AccountPanelParams, opener: HTMLElement | null, dismissDrawer: () => void) => {
      const path = location.pathname;
      const actor = useAuthStore.getState().user?.id;
      const generation = coordinator.generation;
      return () => setTimeout(() => {
        if (router.state.location.pathname !== path || useAuthStore.getState().user?.id !== actor || coordinator.generation !== generation) return;
        dismissDrawer();
        void change("open", params, { panel, opener });
      }, 0);
    },
  }), [current, change, close, coordinator, location.pathname, router]);
}
