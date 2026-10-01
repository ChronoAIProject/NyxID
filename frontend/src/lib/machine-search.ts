import { parseAutomationSearch } from "@/lib/automation-search";

export function parseMachinesSearch(search: Record<string, unknown>): {
  tab?: "logins";
} {
  return { tab: search.tab === "logins" ? ("logins" as const) : undefined };
}

export function parseMachineSetupSearch(search: Record<string, unknown>): {
  setup?: string;
} {
  return { setup: parseAutomationSearch(search).setup };
}

export function parseMachinePairSearch(search: Record<string, unknown>): {
  code?: string;
} {
  return {
    code:
      typeof search.code === "string" && /^[a-z0-9 -]{1,20}$/i.test(search.code)
        ? search.code
        : undefined,
  };
}

export function parseMachineDesktopSearch(search: Record<string, unknown>): {
  conversation_id?: string;
} {
  return {
    conversation_id:
      typeof search.conversation_id === "string" &&
      search.conversation_id.length <= 128
        ? search.conversation_id
        : undefined,
  };
}
