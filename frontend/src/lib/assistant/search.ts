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
 * - `mock` preserves the dev-only HTTP fixture boundary across router writes.
 *
 * `draft` is accepted as a boolean and as the string "true": the router's
 * search parser hands back a real boolean for its own round-tripped links,
 * but a hand-typed or copy-pasted URL arrives as a string.
 */
export interface AssistantSearch {
  readonly c?: string;
  readonly draft?: boolean;
  readonly agent?: string;
  readonly g?: string;
  readonly mock?: 1;
}

export function parseAssistantSearch(
  search: Record<string, unknown>,
): AssistantSearch {
  return {
    ...(typeof search.c === "string" ? { c: search.c } : {}),
    ...(search.draft === true || search.draft === "true"
      ? { draft: true }
      : {}),
    ...(typeof search.agent === "string" && search.agent
      ? { agent: search.agent }
      : {}),
    ...(typeof search.g === "string" && search.g ? { g: search.g } : {}),
    ...(search.mock === 1 || search.mock === "1"
      ? { mock: 1 as const }
      : {}),
  };
}
