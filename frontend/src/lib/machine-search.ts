import { parseAutomationSearch } from "@/lib/automation-search";

export function parseMachinesSearch(search: Record<string, unknown>): {
  tab?: "logins";
  machine?: string;
} {
  return {
    tab: search.tab === "logins" ? ("logins" as const) : undefined,
    ...(typeof search.machine === "string" &&
    /^[a-zA-Z0-9-]{1,128}$/.test(search.machine)
      ? { machine: search.machine }
      : {}),
  };
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
  display?: "secure" | "dev";
} {
  return {
    display: search.display === "dev" ? "dev" : undefined,
    conversation_id:
      typeof search.conversation_id === "string" &&
      search.conversation_id.length <= 128
        ? search.conversation_id
        : undefined,
  };
}
