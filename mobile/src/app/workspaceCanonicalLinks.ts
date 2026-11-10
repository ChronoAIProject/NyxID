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
  const panels = search.getAll("panel");
  // Web chat links (`?c=`, `?g=`, `?agent=`), also the push deeplink shape,
  // and the notifications panel open their thread in the content slab.
  if (
    panels.length === 0 ||
    (panels.length === 1 && panels[0] === "notifications")
  ) {
    const pages = assistantPagesFromSearch(search);
    if (!pages) return withWorkspaceFallback(parse(path));
    if (panels.length === 1) pages.push({ name: "Notifications" });
    return withWorkspaceFallback({
      index: 0,
      routes: [
        {
          name: "Workspace",
          state: { index: pages.length - 1, routes: pages },
        },
      ],
    });
  }
  if (panels.length !== 1 || panels[0] !== "billing")
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

const CONVERSATION_ID = /^nyxa-[a-f0-9]{32}$/;
const GROUP_ID = /^nyxg-[a-fA-F0-9]{32}$/;

/** The content pages a canonical `/assistant` query selects, or undefined
 * when its identifiers are malformed and the ordinary parser should decide. */
function assistantPagesFromSearch(
  search: URLSearchParams,
): { name: string; params?: object }[] | undefined {
  const conversationId = search.get("c");
  const agentId = search.get("agent");
  const groupId = search.get("g");
  const draft = search.get("draft") === "true";
  const validConversation =
    !!conversationId && CONVERSATION_ID.test(conversationId);
  const validGroup = !!groupId && GROUP_ID.test(groupId);
  if (conversationId && !validConversation) return undefined;
  if (groupId && !validGroup) return undefined;
  if (validConversation || agentId || draft) {
    const params: AssistantParams = {
      home: false,
      ...(validConversation ? { conversationId } : {}),
      ...(agentId ? { agentId } : {}),
      draft: !validConversation && draft,
      openLatest: !validConversation && !draft && !!agentId,
    };
    return [{ name: "Assistant", params }];
  }
  if (validGroup)
    return [
      { name: "Assistant", params: assistantHomeParams() },
      { name: "GroupChat", params: { groupId } },
    ];
  return [{ name: "Assistant", params: assistantHomeParams() }];
}
