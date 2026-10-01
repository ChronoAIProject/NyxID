export interface AutomationSearch {
  readonly setup?: string;
  readonly agent?: string;
}

export function parseAutomationSearch(
  search: Record<string, unknown>,
): AutomationSearch {
  return {
    ...(typeof search.setup === "string" &&
    /^[0-9a-f-]{36}$/i.test(search.setup)
      ? { setup: search.setup }
      : {}),
    ...(typeof search.agent === "string" &&
    search.agent.length <= 64 &&
    search.agent
      ? { agent: search.agent }
      : {}),
  };
}
