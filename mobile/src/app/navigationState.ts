import {
  StackRouter,
  type NavigationState,
  type PartialState,
} from "@react-navigation/routers";

export type WorkspaceState = NavigationState | PartialState<NavigationState>;

/** Reset payloads retain route identities while each owning router rehydrates its state. */
export function navigationSnapshot(
  state: WorkspaceState,
): PartialState<NavigationState> {
  return {
    key: state.key,
    stale: true,
    index: state.index,
    routes: state.routes.map(({ state: child, ...route }) => ({
      ...route,
      ...(child ? { state: navigationSnapshot(child) } : {}),
    })),
  };
}

/** Allocate new native route keys without changing a mounted stack's identity. */
export function stackStateWithRoutes(
  previous: WorkspaceState | undefined,
  routes: PartialState<NavigationState>["routes"],
  routeNames: string[],
): NavigationState {
  const state = StackRouter({}).getRehydratedState(
    { stale: true, index: routes.length - 1, routes },
    { routeNames, routeParamList: {}, routeGetIdList: {} },
  );
  return { ...state, key: previous?.key ?? state.key };
}
