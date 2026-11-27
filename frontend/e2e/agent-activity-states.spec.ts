import { mkdir, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { expect, test, type Locator, type Page } from "@playwright/test";
import type { AssistantHttpMockHandler } from "../src/lib/assistant/assistant-http";
import type { NyxAgentHistory } from "../src/schemas/assistant-nyxagent";
import { openAccountMenu } from "./helpers";

type Stage =
  | "idle"
  | "thinking"
  | "working"
  | "drafting"
  | "retry"
  | "waiting"
  | "approval"
  | "approval-only"
  | "stopping"
  | "success"
  | "failure"
  | "cancelled";
type Scenario = {
  thread: string;
  agent: string;
  otherAgent: string;
  stage: Stage;
  turn: number;
  concurrent?: boolean;
  motion?: "still" | "quiet";
  longHistory?: boolean;
};

const base = "/assistant?mock=1&nyxbot=1&avatars=1";
const header = (page: Page) =>
  page.locator("[data-chat-header-controls] [data-avatar-renderer]");

async function setup(page: Page) {
  await page.route("**/api/v1/assistant/nyxagent/cma-sessions?**", (route) =>
    route.fulfill({ json: { sessions: [] } }),
  );
  await page.route("**/api/v1/assistant/nyxagent/machines?**", (route) =>
    route.fulfill({ json: [] }),
  );
  await page.route("**/api/v1/triggers", (route) =>
    route.fulfill({ json: { triggers: [] } }),
  );
  await page.route("**/api/v1/service-preferences", (route) =>
    route.fulfill({ json: { groups: [], version: 0, updated_at: null } }),
  );
  await page.addInitScript(() => {
    const config = () =>
      JSON.parse(
        sessionStorage.getItem("avatar-activity-scenario") ?? "null",
      ) as Scenario | null;
    const listeners = new Set<ReadableStreamDefaultController<Uint8Array>>();
    const encoder = new TextEncoder();
    const frame = (value: object) =>
      encoder.encode("data: " + JSON.stringify(value) + "\n\n");
    document.addEventListener("avatar-fixture-change", () => {
      const state = config();
      if (!state) return;
      for (const listener of listeners)
        listener.enqueue(
          frame({
            type: "conversation",
            id: state.thread,
            group_id: null,
            turn_id: "review-" + state.turn,
            messages: state.turn,
          }),
        );
    });
    const response = (body: unknown) =>
      new Response(JSON.stringify(body), {
        headers: { "content-type": "application/json" },
      });
    const now = "2026-11-26T01:00:00Z";
    const tool = {
      id: "review-tool",
      label: "Find documents",
      status: "running",
      started_at: now,
    };
    let wrapped: AssistantHttpMockHandler | undefined;
    Object.defineProperty(globalThis, "__nyxidAssistantHttpMock", {
      configurable: true,
      get: () => wrapped,
      set(original: AssistantHttpMockHandler) {
        wrapped = async (request) => {
          const { endpoint } = request;
          const state = config();
          const api = "/assistant/nyxagent";
          if (endpoint === api + "/live") {
            let listener: ReadableStreamDefaultController<Uint8Array>;
            return new Response(
              new ReadableStream({
                start(controller) {
                  listener = controller;
                  listeners.add(controller);
                  controller.enqueue(frame({ type: "ready" }));
                },
                cancel() {
                  listeners.delete(listener);
                },
              }),
              { headers: { "content-type": "text/event-stream" } },
            );
          }
          if (endpoint === api + "/agents/activity") {
            return response({
              agents: state?.concurrent
                ? [
                    {
                      agent_id: state.agent,
                      threads: [
                        { conversation_id: state.thread, state: "thinking" },
                        {
                          conversation_id: "background-thread",
                          state: "working",
                        },
                      ],
                    },
                    {
                      agent_id: state.otherAgent,
                      threads: [
                        { conversation_id: "other-thread", state: "waiting" },
                      ],
                    },
                  ]
                : [],
            });
          }
          const result = await original(request);
          if (
            !state ||
            !result?.ok ||
            !endpoint.startsWith(api) ||
            (request.init.method ?? "GET") !== "GET"
          )
            return result;
          if (state.motion && endpoint.split("?")[0] === api + "/agents") {
            const roster = (await result.json()) as {
              agents: Array<{ id: string; avatar?: object | null }>;
            };
            for (const agent of roster.agents) {
              if (agent.id === state.agent)
                agent.avatar = {
                  template: "face-v1",
                  choices: { motion: state.motion },
                };
            }
            return response(roster);
          }
          if (endpoint.split("?")[0] !== api + "/conversations/" + state.thread)
            return result;
          const history = (await result.json()) as NyxAgentHistory;
          const stage = state.stage;
          const turn = "review-" + state.turn;
          const approval = stage === "approval" || stage === "approval-only";
          const settled =
            stage === "success" || stage === "failure" || stage === "cancelled";
          const live =
            !settled && stage !== "idle" && stage !== "approval-only";
          const messages = state.longHistory
            ? [
                ...history.messages,
                ...Array.from({ length: 12 }, (_, index) => ({
                  id: "long-" + index,
                  seq: (history.messages.at(-1)?.seq ?? 0) + index + 1,
                  role: index % 2 ? "assistant" : "user",
                  turn_id: "long-turn-" + Math.floor(index / 2),
                  text: "A review note for the permanent transcript. ".repeat(
                    10,
                  ),
                  status: "completed",
                  error_code: null,
                  created_at: now,
                })),
              ]
            : history.messages;
          return response({
            ...history,
            conversation: {
              ...history.conversation,
              pending_acknowledgements: Number(approval),
              active_turn: live
                ? {
                    turn_id: turn,
                    started_at: now,
                    activities: stage === "working" ? [tool] : [],
                    continuations: stage === "retry" ? 1 : 0,
                    queued: stage === "waiting",
                    stop_requested: stage === "stopping",
                    reply:
                      stage === "drafting"
                        ? {
                            message_id: "review-draft",
                            block_id: "draft",
                            cursor: 1,
                            text: "Here are the findings.",
                          }
                        : null,
                  }
                : null,
            },
            messages: settled
              ? [
                  ...messages,
                  {
                    id: turn + "-reply",
                    role: "assistant",
                    turn_id: turn,
                    seq: (messages.at(-1)?.seq ?? 0) + 1,
                    text:
                      stage === "success"
                        ? "The review is complete."
                        : "The review was interrupted.",
                    status: stage === "success" ? "completed" : "failed",
                    error_code:
                      stage === "failure"
                        ? "review_failed"
                        : stage === "cancelled"
                          ? "cancelled"
                          : null,
                    created_at: now,
                  },
                ]
              : messages,
          });
        };
      },
    });
  });
  await page.goto(base);
  await expect(
    page.getByRole("button", { name: "Create new agent", exact: true }),
  ).toBeVisible();
  const seeded = await page.evaluate(() => {
    const fixture = JSON.parse(
      sessionStorage.getItem("nyxagent-http-fixture-v4")!,
    ) as {
      agents: Array<{ id: string; kind: string; name: string }>;
      rows: Array<[string, { agentId: string }]>;
    };
    const bot = fixture.agents.find((agent) => agent.kind === "nyxbot")!;
    const specialist = fixture.agents.find(
      (agent) => agent.name === "researcher",
    )!;
    return {
      agent: bot.id,
      otherAgent: specialist.id,
      thread: fixture.rows.find(([, row]) => row.agentId === bot.id)![0],
    };
  });
  let state: Scenario = { ...seeded, turn: 1, stage: "idle" };
  const set = async (patch: Partial<Scenario>) => {
    state = { ...state, ...patch };
    await page.evaluate((value) => {
      sessionStorage.setItem("avatar-activity-scenario", JSON.stringify(value));
      document.dispatchEvent(new Event("avatar-fixture-change"));
    }, state);
  };
  const chat = base + "&c=" + seeded.thread;
  await set({});
  await page.goto(chat);
  await expect(header(page)).toHaveAttribute("data-avatar-state", "idle");
  return { set, chat };
}

async function capture(page: Page, name: string) {
  const directory = process.env.AVATAR_ACTIVITY_EVIDENCE;
  if (directory)
    await page.screenshot({ path: join(directory, name + ".png") });
}

async function assertWidth(
  locator: ReturnType<Page["locator"]>,
  width: number,
) {
  await expect
    .poll(async () => (await locator.boundingBox())?.width)
    .toBe(width);
}

for (const theme of ["light", "dark"] as const) {
  test(`chat renders live state transitions in ${theme} and settles a reply once`, async ({
    page,
    context,
  }) => {
    test.setTimeout(90_000);
    await page.setViewportSize({ width: 1440, height: 900 });
    await page.emulateMedia({ colorScheme: theme });
    const scenario = await setup(page);
    await expect(page.locator("html")).toHaveClass(
      new RegExp("theme-" + theme),
    );
    const avatar = header(page);
    await assertWidth(avatar, 56);
    for (const stage of [
      "thinking",
      "working",
      "drafting",
      "retry",
      "waiting",
      "approval",
      "approval-only",
      "stopping",
      "success",
    ] as const) {
      await scenario.set(stage === "retry" ? { stage, turn: 2 } : { stage });
      const expected = stage === "approval-only" ? "approval" : stage;
      await expect(avatar).toHaveAttribute("data-avatar-state", expected);
      await expect(
        page.locator(
          "[data-chat-header-controls] [data-avatar-activity=" + expected + "]",
        ),
      ).toHaveCount(stage === "success" ? 0 : 1);
      await capture(page, "chat-" + theme + "-" + stage);
    }
    await expect(avatar).toHaveAttribute("data-avatar-state", "idle");
    const otherTab = await context.newPage();
    await otherTab.bringToFront();
    await page.bringToFront();
    await expect(avatar).toHaveAttribute("data-avatar-state", "idle");
    await otherTab.close();
    await page.reload();
    await expect(avatar).toHaveAttribute("data-avatar-state", "idle");
    await page
      .getByRole("button", { name: "Home", exact: true })
      .first()
      .click();
    await expect(page.locator("header [data-avatar-renderer]")).toBeVisible();
    await page.goto(scenario.chat);
    await expect(avatar).toHaveAttribute("data-avatar-state", "idle");

    await scenario.set({ stage: "thinking", turn: 3 });
    await expect(avatar).toHaveAttribute("data-avatar-state", "thinking");
    await scenario.set({ stage: "failure" });
    await expect(avatar).toHaveAttribute("data-avatar-state", "failure");
    await capture(page, "chat-" + theme + "-failure");

    await scenario.set({ stage: "stopping", turn: 4 });
    await expect(avatar).toHaveAttribute("data-avatar-state", "stopping");
    await scenario.set({ stage: "cancelled" });
    await expect(avatar).toHaveAttribute("data-avatar-state", "idle");
    await expect(
      page.locator("[data-chat-header-controls] [data-avatar-activity]"),
    ).toHaveCount(0);
    await capture(page, "chat-" + theme + "-cancelled");
  });
}

test("the sidebar and rail aggregate two threads while the header tracks the open one", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  const { set } = await setup(page);
  await set({ stage: "waiting", concurrent: true });
  await expect(header(page)).toHaveAttribute("data-avatar-state", "waiting");
  const sidebar = page.getByRole("navigation", {
    name: "Agents and conversations",
  });
  const bot = sidebar.getByRole("button", { name: /^NyxBot.*Working/ });
  await expect(bot.locator("[data-avatar-activity=working]")).toBeVisible();
  await assertWidth(bot.locator("svg[data-agent-avatar]"), 20);
  await expect(
    sidebar.getByRole("button", { name: /^researcher.*Waiting/ }),
  ).toBeVisible();
  await capture(page, "sidebar-light-concurrent");
  await page.getByRole("button", { name: "Hide sidebar" }).click();
  const rail = page.getByRole("navigation", { name: "Workspace and agents" });
  const railBot = rail.getByRole("button", { name: /^NyxBot.*Working/ });
  await expect(railBot.locator("[data-avatar-activity=working]")).toBeVisible();
  await assertWidth(railBot.locator("svg[data-agent-avatar]"), 24);
  await capture(page, "rail-light-concurrent");
  await page.getByRole("button", { name: "Home", exact: true }).first().click();
  await expect(
    page.locator("header [data-avatar-state=idle]"),
  ).toBeVisible();
  await assertWidth(page.locator("header [data-avatar-renderer]"), 72);
  const personalCard = page
    .locator("li")
    .filter({ has: page.getByText("Your personal agent", { exact: true }) });
  await assertWidth(personalCard.locator("svg[data-agent-avatar]"), 56);
  await capture(page, "home-light-concurrent");
});

test("quiet motion honors visibility, Display, system motion and saved Still", async ({
  page,
}) => {
  test.setTimeout(90_000);
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.emulateMedia({ reducedMotion: "no-preference" });
  const { set } = await setup(page);
  const avatar = header(page);
  const transform = () =>
    avatar.locator("[data-avatar-eye-mask]").getAttribute("d");
  const moves = async () => {
    const initial = await transform();
    await expect.poll(transform, { timeout: 7000 }).not.toBe(initial);
  };
  const rests = async () => {
    const frames = await avatar.evaluate(
      (element) =>
        new Promise<(string | null | undefined)[]>((done) => {
          const values: (string | null | undefined)[] = [];
          const sample = () => {
            values.push(
              element.querySelector("[data-avatar-eye-mask]")?.getAttribute("d"),
            );
            if (values.length === 24) done(values.slice(4));
            else requestAnimationFrame(sample);
          };
          requestAnimationFrame(sample);
        }),
    );
    expect(frames[0]).toBeTruthy();
    expect(new Set(frames).size).toBe(1);
  };
  const display = async () => {
    await openAccountMenu(page);
    await page.getByRole("menuitem", { name: "Settings", exact: true }).click();
    const dialog = page.getByRole("dialog", { name: "Account Settings" });
    await dialog.getByRole("tab", { name: "Display", exact: true }).click();
    return dialog;
  };
  await set({ stage: "working", motion: "quiet" });
  await expect(avatar).toHaveAttribute("data-avatar-state", "working");
  await moves();
  // Headless Chromium keeps documents foregrounded. Exercise the actual
  // visibility listener and scheduler with both browser-reported states.
  const visibility = (value: "visible" | "hidden") =>
    page.evaluate((state) => {
      Object.defineProperty(document, "visibilityState", {
        configurable: true,
        value: state,
      });
      document.dispatchEvent(new Event("visibilitychange"));
    }, value);
  await visibility("hidden");
  await rests();
  await visibility("visible");
  await moves();

  await page.emulateMedia({ reducedMotion: "reduce" });
  await rests();
  await capture(page, "chat-light-system-reduced");
  await page.emulateMedia({ reducedMotion: "no-preference" });
  const dialog = await display();
  await dialog
    .getByRole("radiogroup", { name: "Theme", exact: true })
    .getByRole("radio", { name: "Dark", exact: true })
    .click();
  await dialog.getByRole("radio", { name: "Reduce", exact: true }).click();
  await dialog.getByRole("button", { name: "Close", exact: true }).click();
  await expect(page.locator("html")).toHaveClass(/theme-dark/);
  await rests();
  await capture(page, "chat-dark-display-reduced");
  const motion = await display();
  await motion
    .getByRole("radiogroup", { name: "Motion" })
    .getByRole("radio", { name: "Follow system", exact: true })
    .click();
  await motion.getByRole("button", { name: "Close", exact: true }).click();
  await moves();
  await capture(page, "chat-dark-quiet");

  await set({ motion: "still", turn: 2 });
  await expect(avatar).toHaveAttribute("data-avatar-reaction", "still");
  await rests();
  await capture(page, "chat-dark-saved-still");
  await set({ longHistory: true, turn: 3 });
  const transcript = page.locator("[data-chat-transcript]");
  await expect
    .poll(() =>
      transcript.evaluate(
        (element) => element.scrollHeight - element.clientHeight,
      ),
    )
    .toBeGreaterThan(300);
  await transcript.evaluate((element) => {
    element.scrollTop = 320;
  });
  await expect(page.locator("[data-chat-header-controls]")).toHaveAttribute(
    "data-compact",
    "true",
  );
  await assertWidth(avatar, 20);
  await expect(
    page.locator("[data-chat-header-controls] [data-avatar-activity=working]"),
  ).toBeVisible();
  await capture(page, "chat-dark-compact");
});

const badgeStates = [
  "thinking",
  "working",
  "drafting",
  "waiting",
  "approval",
  "retry",
  "stopping",
  "success",
  "failure",
] as const;
type BadgeState = (typeof badgeStates)[number];

async function cropBadge(
  page: Page,
  surface: Locator,
  width: number,
  name: string,
  state?: BadgeState,
) {
  const face = surface.locator("svg[data-agent-avatar]");
  await assertWidth(face, width);
  const badge = surface.locator("[data-avatar-activity]");
  await expect(badge).toHaveCount(state ? 1 : 0);
  const rendered = state
    ? await badge.evaluate((element) => {
        const bounds = element.getBoundingClientRect();
        const path = getComputedStyle(element.querySelector("path")!);
        const disc = getComputedStyle(element.querySelector("circle")!);
        const ctx = document.createElement("canvas").getContext("2d")!;
        const rgb = (color: string) => {
          ctx.clearRect(0, 0, 1, 1);
          ctx.fillStyle = color;
          ctx.fillRect(0, 0, 1, 1);
          return [...ctx.getImageData(0, 0, 1, 1).data].slice(0, 3);
        };
        return {
          box: {
            x: bounds.x + scrollX,
            y: bounds.y + scrollY,
            width: bounds.width,
            height: bounds.height,
          },
          state: element.getAttribute("data-avatar-activity"),
          tone: element.getAttribute("data-avatar-activity-tone"),
          ink: rgb(path.fill === "none" ? path.stroke : path.fill),
          disc: rgb(disc.fill),
        };
      })
    : null;
  if (rendered) {
    expect(rendered.state).toBe(state);
    expect(rendered.box.width).toBeGreaterThanOrEqual(12);
  }
  const directory = process.env.AVATAR_ACTIVITY_EVIDENCE;
  if (!directory) return;
  const faceBox = (await face.boundingBox())!;
  const badgeBox = rendered?.box ?? faceBox;
  const x = Math.floor(Math.min(faceBox.x, badgeBox.x)) - 2;
  const y = Math.floor(Math.min(faceBox.y, badgeBox.y)) - 2;
  const right =
    Math.ceil(
      Math.max(faceBox.x + faceBox.width, badgeBox.x + badgeBox.width),
    ) + 2;
  const bottom =
    Math.ceil(
      Math.max(faceBox.y + faceBox.height, badgeBox.y + badgeBox.height),
    ) + 2;
  await page.screenshot({
    path: join(directory, name + ".png"),
    clip: { x, y, width: right - x, height: bottom - y },
  });
  if (rendered) {
    return {
      ...rendered,
      name,
      box: { ...badgeBox, x: badgeBox.x - x, y: badgeBox.y - y },
    };
  }
}

for (const theme of ["light", "dark"] as const) {
  for (const dpr of [1, 2]) {
    test.describe(`badge crops in ${theme} at ${dpr}x`, () => {
      test.use({
        colorScheme: theme,
        deviceScaleFactor: dpr,
        reducedMotion: "reduce",
      });

      test("renders each badge at the supported surface sizes", async ({
        page,
      }) => {
        test.setTimeout(120_000);
        await page.emulateMedia({ reducedMotion: "reduce" });
        await page.setViewportSize({ width: 1440, height: 900 });
        const { set } = await setup(page);
        // Wait-free crops use the final layout of the reduced-motion surfaces.
        await page.addStyleTag({
          content:
            "*, *::before, *::after { animation-duration: 0s !important; transition-duration: 0s !important; }",
        });
        await expect(page.locator("html")).toHaveClass(
          new RegExp("theme-" + theme),
        );
        const prefix = `badge-${theme}-dpr${dpr}`;
        const sidebar = page
          .getByRole("navigation", { name: "Agents and conversations" })
          .getByRole("button", { name: /^NyxBot/ })
          .first();
        const rail = page
          .getByRole("navigation", { name: "Workspace and agents" })
          .getByRole("button", { name: /^NyxBot/ })
          .first();
        const controls = page.locator("[data-chat-header-controls]");
        const details = page.getByRole("dialog", { name: "Agent details" });
        const transcript = page.locator("[data-chat-transcript]");
        const samples = [];
        for (const [index, stage] of badgeStates.entries()) {
          const turn = index + 2;
          const settled = stage === "success" || stage === "failure";
          if (settled) {
            await set({ stage: "thinking", turn, longHistory: true });
            await expect(header(page)).toHaveAttribute(
              "data-avatar-state",
              "thinking",
            );
          } else {
            await set({ stage, turn, longHistory: true });
            await expect(header(page)).toHaveAttribute(
              "data-avatar-state",
              stage,
            );
          }
          await expect
            .poll(() =>
              transcript.evaluate(
                (element) => element.scrollHeight - element.clientHeight,
              ),
            )
            .toBeGreaterThan(320);
          await transcript.evaluate((element) => {
            element.scrollTop = 320;
          });
          await expect(controls).toHaveAttribute("data-compact", "true");
          if (settled) await set({ stage });
          await expect(header(page)).toHaveAttribute(
            "data-avatar-state",
            stage,
          );
          samples.push(
            await cropBadge(
              page,
              controls,
              20,
              `${prefix}-${stage}-chat`,
              stage === "success" ? undefined : stage,
            ),
          );

          // The roster ranks only live thread states, even while chat displays a settlement.
          const rosterState = settled ? undefined : stage;
          samples.push(
            await cropBadge(
              page,
              sidebar,
              20,
              `${prefix}-${stage}-sidebar`,
              rosterState,
            ),
          );
          await page.getByRole("button", { name: "Hide sidebar" }).click();
          samples.push(
            await cropBadge(
              page,
              rail,
              24,
              `${prefix}-${stage}-rail`,
              rosterState,
            ),
          );
          await page.getByRole("button", { name: "Show sidebar" }).click();
          await controls.getByRole("button", { name: "Agent details" }).click();
          samples.push(
            await cropBadge(
              page,
              details.locator(".agent-header-avatar"),
              112,
              `${prefix}-${stage}-details`,
              rosterState,
            ),
          );
          await details
            .getByRole("button", { name: "Close agent details" })
            .click();
          await expect(details).toBeHidden();
          await expect(details).toHaveCount(0);
        }
        const directory = process.env.AVATAR_ACTIVITY_EVIDENCE;
        if (directory) {
          await mkdir(directory, { recursive: true });
          await writeFile(
            join(directory, prefix + "-samples.json"),
            JSON.stringify(samples.filter(Boolean), null, 2),
          );
        }
      });
    });
  }
}
