import {
  CommonActions,
  type NavigationState,
  type PartialState,
} from "@react-navigation/routers";
import type { AssistantParams } from "./navigationTypes";
import { navigationSnapshot, stackStateWithRoutes } from "./navigationState";
import { workspaceContentRouteNames } from "./navigationTypes";
import {
  assistantHomeParams,
  newAssistantChatParams,
} from "./assistantChatParams";

export type WorkspaceChatTarget =
  | { kind: "home" }
  | { kind: "agent"; agentId?: string; conversationId?: string }
  | { kind: "group"; groupId: string };

type State = PartialState<NavigationState>;
/** Select an explicit chat/Home without replacing the mounted Assistant key. */
export function workspaceChatNavigationAction(
  state: NavigationState | undefined,
  target: WorkspaceChatTarget,
) {
  const workspace = state?.routes.find((route) => route.name === "Workspace");
  if (!state || !workspace) return null;
  const previous = workspace.state
    ? navigationSnapshot(workspace.state)
    : undefined;
  const chat = previous?.routes.find((route) => route.name === "Assistant") ?? {
    name: "Assistant",
    params: assistantHomeParams(),
  };
  let routes: State["routes"];
  if (target.kind === "group") {
    const prior = previous?.routes.find(
      (route) =>
        route.name === "GroupChat" &&
        (route.params as { groupId?: string } | undefined)?.groupId ===
          target.groupId,
    );
    routes = [
      chat,
      { ...prior, name: "GroupChat", params: { groupId: target.groupId } },
    ];
  } else {
    const params =
      target.kind === "home"
        ? assistantHomeParams()
        : target.conversationId
          ? {
              home: false,
              agentId: target.agentId,
              conversationId: target.conversationId,
              acknowledgementId: undefined,
              draft: false,
              openLatest: false,
            }
          : newAssistantChatParams(target.agentId);
    routes = [{ ...chat, params }];
  }
  return {
    ...CommonActions.reset({
      ...state,
      index: 0,
      routes: [
        {
          ...workspace,
          params: undefined,
          state: stackStateWithRoutes(
            previous,
            routes,
            workspaceContentRouteNames,
          ),
        },
      ],
    }),
    target: state.key,
  };
}

/** A contextual chat pushes above its actual caller, retaining every draft/editor. */
export function workspaceContextChatNavigationAction(
  state: NavigationState | undefined,
  params: AssistantParams,
) {
  if (!state) return null;
  const workspace = state.routes.find((route) => route.name === "Workspace");
  const candidate = workspace
    ? workspace.state
    : state.routeNames.includes("Assistant")
      ? state
      : undefined;
  const content = candidate ? navigationSnapshot(candidate) : undefined;
  if (!content) return null;
  const existing =
    params.conversationId && !params.draft
      ? content.routes.findLastIndex(
          (route) =>
            (route.name === "Assistant" || route.name === "AssistantContext") &&
            (route.params as AssistantParams | undefined)?.conversationId ===
              params.conversationId,
        )
      : -1;
  const routes =
    existing !== -1
      ? content.routes
          .slice(0, existing + 1)
          .map((route, index) =>
            index === existing ? { ...route, params } : route,
          )
      : [
          ...content.routes.slice(
            0,
            (content.index ?? content.routes.length - 1) + 1,
          ),
          { name: "AssistantContext", params },
        ];
  const nested = stackStateWithRoutes(
    content,
    routes,
    workspaceContentRouteNames,
  );
  return {
    ...CommonActions.reset(
      workspace
        ? {
            ...state,
            index: 0,
            routes: [{ ...workspace, params: undefined, state: nested }],
          }
        : nested,
    ),
    target: state.key,
  };
}

export function activeWorkspaceChatTarget(
  route: { name: string; params?: object } | undefined,
): WorkspaceChatTarget | undefined {
  if (!route) return undefined;
  const params = route.params as Record<string, unknown> | undefined;
  const stringParam = (name: string) =>
    typeof params?.[name] === "string" && params[name].length
      ? (params[name] as string)
      : undefined;
  if (route.name === "Assistant" || route.name === "AssistantContext") {
    if (
      route.name === "Assistant" &&
      (!params ||
        (params.home === true &&
          !params.conversationId &&
          !params.agentId &&
          !params.draft &&
          !params.openLatest))
    )
      return { kind: "home" };
    return {
      kind: "agent",
      agentId: stringParam("agentId"),
      conversationId: stringParam("conversationId"),
    };
  }
  const groupId = stringParam("groupId");
  if (route.name === "GroupChat" && groupId) return { kind: "group", groupId };
  return undefined;
}
