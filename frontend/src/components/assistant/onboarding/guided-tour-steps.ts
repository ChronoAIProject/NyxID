export interface TourStep {
  /** The `data-tour` value of the element to spotlight. */
  readonly target: string;
  readonly title: string;
  readonly body: string;
}

/** The assistant screen, in reading order. */
export const ASSISTANT_TOUR: readonly TourStep[] = [
  {
    target: "composer",
    title: "Ask NyxBot anything",
    body: "Type a request in plain words. The menu on the right sets how often NyxBot asks before it acts.",
  },
  {
    target: "nyxbot-home-next-steps",
    title: "Your next steps",
    body: "One click hands a step to NyxBot. Dismiss the ones you don't need.",
  },
  {
    target: "agents",
    title: "NyxBot and your specialists",
    body: "NyxBot is your chief of staff. Specialists you create appear here too; open one to chat with it directly.",
  },
  {
    target: "create-agent",
    title: "Create a specialist",
    body: "Give it a job and only the access it needs. Put several in a crew to work together.",
  },
  {
    target: "approvals",
    title: "Approvals",
    body: "Requests that need your decision wait here, and NyxBot continues once you decide.",
  },
  {
    target: "automations",
    title: "Automations",
    body: "Schedules and webhooks NyxBot runs for you, like a daily brief.",
  },
];
