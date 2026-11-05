import type { NavigationState, PartialState } from "@react-navigation/routers";
import { withWorkspaceFallback } from "./navigationHierarchy";
import type { AssistantParams } from "./navigationTypes";
import {
  assistantHomeParams,
  newAssistantChatParams,
} from "./assistantChatParams";

type LinkState = PartialState<NavigationState> | undefined;

/** Web account panels share /assistant with chat, so query state selects the drawer. */
export function workspaceStateFromPath(
  path: string,
  parse: (path: string) => LinkState,
): LinkState {
  let url: URL;
  let search: URLSearchParams;
  try {
    url = new URL(path, "https://native.nyxid.invalid/");
    // Removed native authoring URLs lead to the existing read-only pages.
    const automation =
      /^\/(?:assistant\/)?automations(?:\/edit(?:\/([^/]+))?|\/preferences)?\/?$/.exec(
        url.pathname,
      );
    if (automation) {
      const id = automation[1];
      const uuid =
        /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;
      const agentId =
        url.searchParams.get("agentId") ?? url.searchParams.get("agent");
      const query = agentId ? `?${new URLSearchParams({ agentId })}` : "";
      return withWorkspaceFallback(
        parse(
          id && uuid.test(id) ? `automations/${id}` : `automations${query}`,
        ),
      );
    }
    if (!/^\/assistant\/?$/.test(url.pathname))
      return withWorkspaceFallback(parse(path));
    search = url.searchParams;
  } catch {
    return withWorkspaceFallback(parse(path));
  }
  if (
    !search.has("panel") &&
    !search.has("c") &&
    !search.has("g") &&
    !search.has("agent")
  ) {
    const state = withWorkspaceFallback(
      parse("chat?draft=true&openLatest=false"),
    );
    if (!state) return state;
    return {
      ...state,
      routes: state.routes.map((route) =>
        route.name === "Workspace"
          ? {
              ...route,
              state: {
                routes: [
                  {
                    name: "Assistant",
                    params:
                      search.get("draft") === "true"
                        ? newAssistantChatParams()
                        : assistantHomeParams(),
                  },
                ],
              },
            }
          : route,
      ),
    };
  }
  if (search.getAll("panel").length !== 1 || search.get("panel") !== "billing")
    return withWorkspaceFallback(parse(path));

  const period = search.get("panelPeriod");
  const state = withWorkspaceFallback(
    parse(
      `account/usage${period && ["24h", "7d", "30d"].includes(period) ? `?period=${period}` : ""}`,
    ),
  );
  if (!state) return state;

  // Only supported read filters cross the native boundary. Purchase actions do not.
  const conversationId = search.get("c");
  const agentId = search.get("agent");
  const groupId = search.get("g");
  const draft = search.get("draft") === "true";
  const validConversation =
    !!conversationId && /^nyxa-[a-f0-9]{32}$/.test(conversationId);
  const validGroup = !!groupId && /^nyxg-[a-fA-F0-9]{32}$/.test(groupId);
  const params: AssistantParams = {
    ...(validConversation ? { conversationId } : {}),
    ...(agentId ? { agentId } : {}),
    draft: !validConversation && draft,
    openLatest: !validConversation && !draft && !!agentId,
  };
  const routes = state.routes.map((route) =>
    route.name === "Workspace" && (validConversation || agentId || draft)
      ? { ...route, state: { routes: [{ name: "Assistant", params }] } }
      : route,
  );
  if (!validConversation && !draft && !agentId && validGroup) {
    const workspace = routes.find((route) => route.name === "Workspace");
    if (workspace)
      workspace.state = {
        index: 1,
        routes: [
          { name: "Assistant", params: assistantHomeParams() },
          { name: "GroupChat", params: { groupId } },
        ],
      };
  }
  return { ...state, index: routes.length - 1, routes };
}
