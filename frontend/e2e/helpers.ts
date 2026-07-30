import { expect, type Page } from "@playwright/test";
import type { AssistantMockFaults } from "../src/lib/assistant/transport";

/**
 * Shared driver for the assistant chat flow specs.
 *
 * Everything here addresses the screen the way a user (or their screen
 * reader) sees it: roles, visible text, and the three dedicated state markers
 * the thread already exposes (`data-assistant-halo`, `data-streaming-dots`,
 * `data-empty-turn-error`). No selectors reach into implementation classes.
 */

/** Seeded mock conversations (src/lib/assistant/mock-data.ts). */
export const SEEDED = {
  stripe: {
    id: "conversation-stripe",
    title: "Failed Stripe payments digest",
  },
  github: { id: "conversation-github", title: "Rotate GitHub deploy key" },
  weekly: { id: "conversation-weekly", title: "Weekly usage report" },
} as const;

/** Final text every scripted mock turn streams (createScriptedTurn). */
export const SCRIPTED_REPLY_START = "I checked the current conversation";
export const SCRIPTED_TOOL_RESULT = "Posted to #payments-oncall";

export async function openAssistant(
  page: Page,
  options: { conversation?: string; faults?: AssistantMockFaults } = {},
): Promise<void> {
  if (options.faults) {
    const faults = options.faults;
    await page.addInitScript((value: AssistantMockFaults) => {
      (
        window as { __assistantMockFaults?: AssistantMockFaults }
      ).__assistantMockFaults = value;
    }, faults);
  }
  const search = new URLSearchParams({ mock: "1" });
  if (options.conversation) search.set("c", options.conversation);
  await page.goto(`/assistant?${search.toString()}`);
  await expect(
    page.getByRole("button", { name: "New chat" }),
  ).toBeVisible({ timeout: 20_000 });
}

/** The chat surface (thread + composer), excluding sidebar and header. */
export function thread(page: Page) {
  return page.getByRole("main");
}

export function composerInput(page: Page) {
  // Placeholder flips to "Assistant is working..." while a turn runs; match
  // the element, not one placeholder value.
  return page.locator("textarea");
}

export async function sendMessage(page: Page, text: string): Promise<void> {
  const input = composerInput(page);
  await input.fill(text);
  await input.press("Enter");
}

export function streamingDots(page: Page) {
  return page.locator("[data-streaming-dots]");
}

export function assistantHalo(page: Page) {
  return page.locator("[data-assistant-halo]");
}

export function emptyTurnError(page: Page) {
  return page.locator("[data-empty-turn-error]");
}

export function stopButton(page: Page) {
  return page.getByRole("button", { name: "Stop assistant turn" });
}

export function sendButton(page: Page) {
  return page.getByRole("button", { name: "Send message" });
}

/**
 * Sidebar row select-button for a conversation. `exact` so the row's
 * "Options for <title>" menu trigger can never match instead.
 */
export function conversationRow(page: Page, title: string) {
  return page
    .getByRole("navigation")
    .getByRole("button", { name: title, exact: true });
}

export interface EpisodeSnapshot {
  readonly open: boolean;
  readonly printed: boolean;
  readonly projecting: boolean;
}

/**
 * Read a per-conversation cache slot through the dev-only query client
 * handle (src/main.tsx). Used ONLY where the defect under audit is a state
 * leak nothing currently renders; flow specs stay on-screen-only.
 */
export async function readEpisode(
  page: Page,
  conversationId: string,
): Promise<EpisodeSnapshot | null> {
  return page.evaluate((id) => {
    const client = (
      window as {
        __nyxQueryClient?: {
          getQueryData: (key: readonly unknown[]) => unknown;
        };
      }
    ).__nyxQueryClient;
    return (client?.getQueryData(["assistant", "episode", id]) ??
      null) as EpisodeSnapshot | null;
  }, conversationId);
}
