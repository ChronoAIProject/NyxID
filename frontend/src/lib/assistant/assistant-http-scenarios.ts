export interface AssistantHttpScenario {
  readonly id: string;
  readonly pattern: RegExp;
  readonly reply: string;
  readonly serviceSlug?: string;
}

export const assistantHttpScenarios: readonly AssistantHttpScenario[] = [
  {
    id: "connect-github",
    pattern: /connect (to )?(my )?github/i,
    reply: "I can prepare a brokered GitHub connection for this account.",
    serviceSlug: "api-github",
  },
  {
    id: "github-issues",
    pattern: /(what|show|list).*(gh|github).*issues/i,
    reply: "You have 7 open issues on acme/web. Two have not moved in 30 days.",
    serviceSlug: "api-github",
  },
  {
    id: "github-issues-repo",
    pattern: /issues (?:in|on) (\S+)/i,
    reply: "The requested repository has 3 open issues.",
    serviceSlug: "api-github",
  },
  {
    id: "approval-demo",
    pattern: /post .*digest/i,
    reply: "The digest is ready for the typed approval flow.",
  },
  {
    id: "error-demo",
    pattern: /simulate (an? )?error/i,
    reply: "The mock scenario failed as requested.",
  },
];

export function matchAssistantHttpScenario(
  content: string,
  disabledScenarioIds: readonly string[],
): AssistantHttpScenario | null {
  for (const scenario of assistantHttpScenarios) {
    if (
      !disabledScenarioIds.includes(scenario.id) &&
      scenario.pattern.test(content)
    ) {
      return scenario;
    }
  }
  return null;
}
