import { withAccountPanelSearch, type AccountPanelSearch } from "./account-panel-search";

/**
 * The `/assistant` search contract, in one place: the router validates with
 * it and the page reads with it, so the two cannot drift into disagreeing
 * about what a draft is.
 *
 * - `c`     addresses a conversation.
 * - `draft` is the pre-provision "New chat" state. The button paints an
 *   empty thread under `?draft` immediately, then swaps in `?c=<id>` once
 *   the actor lands (see AssistantPage.createNewChat).
 * - `agent` selects a NyxBot agent: with `draft` it is the agent a new
 *   thread starts with; alone it lands on that agent's latest thread.
 * - `g`     opens a NyxBot group chat. With no `c`, `draft`, `agent` or `g`
 *   the NyxAgent engine shows the NyxBot home.
 * - `details` opens the agent details sheet on one section (`grants` or
 *   `skills`) for `agent`, else the open thread's agent. The sheet is URL
 *   state, so a server link (`AssistantPage::AgentDetails`) survives reload
 *   and Back closes it; an inaccessible or destroyed agent shows an explicit
 *   no-access state instead of falling back to NyxBot.
 * - `mock` preserves the dev-only HTTP fixture boundary across router writes.
 *
 * `draft` is accepted as a boolean and as the string "true": the router's
 * search parser hands back a real boolean for its own round-tripped links,
 * but a hand-typed or copy-pasted URL arrives as a string.
 */
export const AGENT_DETAILS_TABS = ["grants", "skills"] as const;
export type AgentDetailsTab = (typeof AGENT_DETAILS_TABS)[number];

export interface AssistantSearch extends AccountPanelSearch {
  readonly c?: string;
  readonly card?: string;
  readonly proposal?: string;
  readonly draft?: boolean;
  readonly agent?: string;
  readonly details?: AgentDetailsTab;
  readonly g?: string;
  readonly mock?: 1;
}

function agentDetailsTab(value: unknown): AgentDetailsTab | undefined {
  return AGENT_DETAILS_TABS.find((tab) => tab === value);
}

function parseChatSearch(
  search: Record<string, unknown>,
): Omit<AssistantSearch, keyof AccountPanelSearch> {
  return {
    ...(typeof search.c === "string" ? { c: search.c } : {}),
    ...(typeof search.card === "string" && /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(search.card)
      ? { card: search.card }
      : {}),
    ...(typeof search.proposal === "string" && /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(search.proposal)
      ? { proposal: search.proposal } : {}),
    ...(search.draft === true || search.draft === "true"
      ? { draft: true }
      : {}),
    ...(typeof search.agent === "string" && search.agent
      ? { agent: search.agent }
      : {}),
    ...(agentDetailsTab(search.details) !== undefined
      ? { details: agentDetailsTab(search.details) }
      : {}),
    ...(typeof search.g === "string" && search.g ? { g: search.g } : {}),
    ...(search.mock === 1 || search.mock === "1"
      ? { mock: 1 as const }
      : {}),
  };
}

export const parseAssistantSearch = withAccountPanelSearch(parseChatSearch);
