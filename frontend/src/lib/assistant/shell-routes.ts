export const ASSISTANT_SHELL_ROUTES = [
  "/assistant",
  "/assistant/plugins",
  "/assistant/approvals",
  "/assistant/automations",
  "/assistant/machines",
  "/assistant/machines/new",
  "/assistant/machines/pair",
] as const;
export type AssistantShellRoute = (typeof ASSISTANT_SHELL_ROUTES)[number];

export function isAssistantShellRoute(path: string): path is AssistantShellRoute {
  return ASSISTANT_SHELL_ROUTES.some((route) => route === path);
}

