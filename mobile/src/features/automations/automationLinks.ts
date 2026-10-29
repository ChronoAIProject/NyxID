export type AutomationLinkParams = {
  agentId?: string;
};

export type AutomationLink =
  | { kind: "automation"; params: AutomationLinkParams }
  | { kind: "invalid"; message: string };

const setupId =
  /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;

/** Model text may link into native Automations only on the configured frontend. */
export function parseAutomationLink(
  value: string,
  frontendUrl: string,
): AutomationLink | null {
  try {
    const frontend = new URL(frontendUrl);
    const link = new URL(value);
    if (
      frontend.protocol !== "https:" ||
      frontend.username ||
      frontend.password ||
      link.protocol !== "https:" ||
      link.username ||
      link.password ||
      link.origin !== frontend.origin ||
      !["/assistant/automations", "/automations"].includes(link.pathname)
    )
      return null;

    const params: AutomationLinkParams = {};
    const setup = link.searchParams.getAll("setup");
    const agent = link.searchParams.getAll("agent");
    if (setup.length > 1 || (setup.length && !setupId.test(setup[0]!)))
      return {
        kind: "invalid",
        message:
          "This automation setup link is invalid. Ask NyxBot for a new link.",
      };
    if (
      agent.length > 1 ||
      (agent.length &&
        (!agent[0]!.trim() ||
          agent[0]!.length > 64 ||
          /[\u0000-\u001f\u007f]/.test(agent[0]!)))
    )
      return {
        kind: "invalid",
        message:
          "This automation link has an invalid agent. Ask NyxBot for a new link.",
      };
    if (agent.length) params.agentId = agent[0]!;
    // Setup authoring is not native. Only the read-only agent filter crosses.
    // Instructions, secrets, and arbitrary return URLs never enter route state.
    return { kind: "automation", params };
  } catch {
    return null;
  }
}
