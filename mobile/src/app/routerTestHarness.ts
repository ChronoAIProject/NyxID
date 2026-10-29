import type { NavigationState, PartialState, NavigationAction } from "@react-navigation/routers";

// Production navigators rehydrate partial action results before rendering.
export function hydratedTestRouter<R extends {
  getInitialState: (...args: any[]) => NavigationState;
  getStateForAction: (...args: any[]) => any;
  getRehydratedState: (...args: any[]) => NavigationState;
}>(router: R): Omit<R, "getStateForAction" | "getRehydratedState"> & {
  getRehydratedState: (state: NavigationState | PartialState<NavigationState>, options: Parameters<R["getRehydratedState"]>[1]) => ReturnType<R["getInitialState"]>;
  getStateForAction: (state: NavigationState | PartialState<NavigationState>, action: NavigationAction, options: Parameters<R["getStateForAction"]>[2]) => ReturnType<R["getInitialState"]> | null;
} {
  type State = ReturnType<R["getInitialState"]>;
  const rehydrate = (state: NavigationState | PartialState<NavigationState>, options: Parameters<R["getRehydratedState"]>[1]): State => router.getRehydratedState(state, options) as State;
  return {
    ...router,
    getRehydratedState: rehydrate,
    getStateForAction(state, action, options) {
      const next = router.getStateForAction(rehydrate(state, options), action, options);
      return next ? rehydrate(next, options) : null;
    },
  };
}
