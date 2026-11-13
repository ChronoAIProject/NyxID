import type {
  NavigationAction,
  NavigationState,
  PartialState,
} from "@react-navigation/routers";
import { getActionFromState, getStateFromPath } from "@react-navigation/core";
import { withWorkspaceFallback } from "./navigationHierarchy";
import { navigationLinkingConfig } from "./navigationLinking";
import type { AssistantParams } from "./navigationTypes";
import {
  assistantHomeParams,
  newAssistantChatParams,
} from "./assistantChatParams";

type LinkState = PartialState<NavigationState> | undefined;

/**
 * What opening a first-party path inside the app should dispatch: the same
 * state a tapped push or universal link produces, as a navigate action when
 * the linking config yields one, else a root reset. Undefined when the path
 * names no native page.
 */
export function canonicalLinkNavigation(
  path: string,
):
  | { type: "action"; action: NavigationAction }
  | { type: "reset"; state: PartialState<NavigationState> }
  | undefined {
  const state = workspaceStateFromPath(path, (next) =>
    getStateFromPath(next, navigationLinkingConfig),
  );
  if (!state) return undefined;
  const action = getActionFromState(state, navigationLinkingConfig);
  return action ? { type: "action", action } : { type: "reset", state };
}

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
    if (/^\/assistant\/approvals\/?$/.test(url.pathname))
      return withWorkspaceFallback(parse("activity"));
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
const CARD_ID = /^[A-Za-z0-9][A-Za-z0-9._:-]{0,127}$/;

/** The content pages a canonical `/assistant` query selects, or undefined
 * when its identifiers are malformed and the ordinary parser should decide.
 * Server links (`AssistantPage`) name a crew with `?c=` as well as `?g=`,
 * focus a decision with `?card=` and open an agent's details with
 * `?agent=&details=`. */
function assistantPagesFromSearch(
  search: URLSearchParams,
): { name: string; params?: object }[] | undefined {
  const c = search.get("c");
  const agentId = search.get("agent");
  const g = search.get("g");
  const card = search.get("card");
  const draft = search.get("draft") === "true";
  const conversationId = c && CONVERSATION_ID.test(c) ? c : undefined;
  const groupId = g && GROUP_ID.test(g) ? g : c && GROUP_ID.test(c) ? c : undefined;
  if (c && !conversationId && c !== groupId) return undefined;
  if (g && !groupId) return undefined;
  if (card && !CARD_ID.test(card)) return undefined;
  if (groupId && !conversationId && !draft)
    return [
      { name: "Assistant", params: assistantHomeParams() },
      { name: "GroupChat", params: { groupId } },
    ];
  if (agentId && search.has("details") && !conversationId && !draft)
    return [
      { name: "Assistant", params: assistantHomeParams() },
      { name: "Agents" },
      { name: "AgentDetail", params: { agentId } },
    ];
  if (conversationId || agentId || draft) {
    const params: AssistantParams = {
      home: false,
      ...(conversationId ? { conversationId } : {}),
      ...(conversationId && card ? { acknowledgementId: card } : {}),
      ...(agentId ? { agentId } : {}),
      draft: !conversationId && draft,
      openLatest: !conversationId && !draft && !!agentId,
    };
    return [{ name: "Assistant", params }];
  }
  return [{ name: "Assistant", params: assistantHomeParams() }];
}
