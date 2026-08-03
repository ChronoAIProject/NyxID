import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { applyTurnEvent, EMPTY_TURN_STATE } from "@/lib/assistant/stream";
import type { ActionReport } from "@/schemas/assistant-actions";
import type {
  ActionCardContentBlock,
  ApprovalCardContentBlock,
  TurnEvent,
  TurnReducerState,
} from "@/types/assistant";
import {
  ScenarioConfigError,
  ScenarioEngine,
  compileScenarioSet,
  flow,
  scenario,
  type CursorSource,
  type ScenarioDefinition,
  type ScenarioFlow,
  type WorldPort,
} from "./scenario-engine";

class TestWorld implements WorldPort {
  readonly connected = new Set<string>();

  isConnected(serviceSlug: string): boolean {
    return this.connected.has(serviceSlug);
  }

  connect(serviceSlug: string): void {
    this.connected.add(serviceSlug);
  }

  disconnect(serviceSlug: string): void {
    this.connected.delete(serviceSlug);
  }
}

class TestCursors implements CursorSource {
  private readonly cursors = new Map<string, number>();

  constructor(initial = 0) {
    this.initial = initial;
  }

  private readonly initial: number;

  next(conversationId: string): number {
    const cursor = (this.cursors.get(conversationId) ?? this.initial) + 1;
    this.cursors.set(conversationId, cursor);
    return cursor;
  }
}

function createEngine(
  scenarios: readonly ScenarioDefinition[],
  flows: Readonly<Record<string, ScenarioFlow>> = {},
  options: {
    readonly world?: TestWorld;
    readonly cursors?: CursorSource;
    readonly now?: () => number;
  } = {},
): { readonly engine: ScenarioEngine; readonly world: TestWorld } {
  const world = options.world ?? new TestWorld();
  return {
    engine: new ScenarioEngine(
      compileScenarioSet(scenarios, flows),
      world,
      options.cursors,
      { now: options.now },
    ),
    world,
  };
}

function matchOrThrow(engine: ScenarioEngine, content = "go") {
  const match = engine.match(content);
  if (!match) throw new Error("Expected scenario to match.");
  return match;
}

async function finishTimers(): Promise<void> {
  await vi.runAllTimersAsync();
}

function completedBlock<T extends "action_card" | "approval_card">(
  events: readonly TurnEvent[],
  type: T,
): Extract<ActionCardContentBlock | ApprovalCardContentBlock, { type: T }> {
  const block = events.find(
    (event) => event.event === "block.completed" && event.block.type === type,
  );
  if (block?.event !== "block.completed" || block.block.type !== type) {
    throw new Error(`Expected ${type} block.`);
  }
  return block.block as Extract<
    ActionCardContentBlock | ApprovalCardContentBlock,
    { type: T }
  >;
}

function actionReport(
  card: ActionCardContentBlock,
  disposition: ActionReport["disposition"],
): ActionReport {
  return {
    actionRequestId: card.action_request_id,
    originTurnId: card.origin_turn_id,
    disposition,
    ...(disposition === "completed"
      ? { resource: { userService: { userServiceId: "service-1" } } }
      : {}),
  };
}

describe("ScenarioEngine compilation", () => {
  it("builds text, shared tool-run, artifact, and completed terminal events", async () => {
    const definition = scenario("complete", /go/i, (script) =>
      script
        .say("A".repeat(45))
        .tool("github.first", "done")
        .toolFail("github.second", "failed")
        .artifact("issues.txt", "one\ntwo"),
    );
    const { engine } = createEngine([definition]);
    const events: TurnEvent[] = [];

    engine.play("conversation", matchOrThrow(engine), (event) =>
      events.push(event),
    );
    await finishTimers();

    expect(events[0]).toMatchObject({
      event: "turn.status",
      status: "running",
    });
    expect(
      events.filter((event) => event.event === "block.delta"),
    ).toHaveLength(2);
    const runStarts = events.filter(
      (event) => event.event === "block.started" && event.block.type === "run",
    );
    expect(runStarts).toHaveLength(1);
    expect(runStarts[0]).toMatchObject({
      block: { steps_total: 2 },
    });
    expect(events).toContainEqual(
      expect.objectContaining({
        event: "turn.completed",
        status: "completed",
        error: null,
      }),
    );
    const started = events
      .filter((event) => event.event === "block.started")
      .map((event) => event.block_id);
    const completed = events
      .filter((event) => event.event === "block.completed")
      .map((event) => event.block_id);
    expect(completed).toEqual(expect.arrayContaining(started));
  });

  it("emits a named failed terminal after fail", async () => {
    const { engine } = createEngine([
      scenario("failure", /go/, (script) =>
        script.say("Starting").fail("mock_error", "Expected failure."),
      ),
    ]);
    const events: TurnEvent[] = [];

    engine.play("conversation", matchOrThrow(engine), (event) =>
      events.push(event),
    );
    await finishTimers();

    expect(events.slice(-2)).toEqual([
      expect.objectContaining({ event: "turn.status", status: "failed" }),
      expect.objectContaining({
        event: "turn.completed",
        status: "failed",
        error: { code: "mock_error", message: "Expected failure." },
      }),
    ]);
  });

  it("rejects unknown flow references", () => {
    expect(() =>
      compileScenarioSet(
        [scenario("unknown", /go/, (script) => script.run("missing"))],
        {},
      ),
    ).toThrowError(new ScenarioConfigError("Unknown flow reference: missing"));
  });

  it("rejects recursive flow references", () => {
    expect(() =>
      compileScenarioSet([], {
        recursive: flow((script) => script.run("recursive")),
      }),
    ).toThrow(/Flow recursion is not allowed/);
  });

  it("rejects more than one resumable card at an await boundary (F10)", () => {
    expect(() =>
      compileScenarioSet(
        [
          scenario("two-cards", /go/, (script) =>
            script
              .action("service.connect", { service: "api-github" })
              .approval("api-github", "Post the digest?")
              .await(),
          ),
        ],
        {},
      ),
    ).toThrow(/cannot park more than one resumable card \(F10\)/);
  });

  it("rejects a branch card composed with a card after await (F10)", () => {
    expect(() =>
      compileScenarioSet(
        [
          scenario("two-park", /go/, (script) =>
            script
              .action("service.connect", { service: "api-github" })
              .await({
                completed: (branch) =>
                  branch.action("service.connect", {
                    service: "api-openai",
                  }),
              })
              .action("service.connect", { service: "api-lark" })
              .await(),
          ),
        ],
        {},
      ),
    ).toThrow(/cannot park more than one resumable card \(F10\)/);
  });

  it("validates action params and registry support during config load", () => {
    expect(() =>
      compileScenarioSet(
        [
          scenario("bad-action", /go/, (script) =>
            script.action("service.typo", { service: "api-github" }),
          ),
        ],
        {},
      ),
    ).toThrow(/Action service\.typo is not supported/);
  });
});

describe("ScenarioEngine continuations and world", () => {
  it("ends a parked action segment waiting then blocked and resumes in a new turn", async () => {
    const definition = scenario("action", /go/, (script) =>
      script
        .say("Connect first")
        .action("service.connect", {
          service: "api-github",
          scopes: ["repo"],
        })
        .await({
          completed: (branch) => branch.say("Connected branch"),
          declined: (branch) => branch.say("Declined branch"),
          failed: (branch) => branch.say("Failed branch"),
        })
        .connect("api-github")
        .say("After await"),
    );
    const { engine, world } = createEngine([definition]);
    const firstEvents: TurnEvent[] = [];
    const continuationEvents: TurnEvent[] = [];

    const firstHandle = engine.play(
      "conversation",
      matchOrThrow(engine),
      (event) => firstEvents.push(event),
    );
    await finishTimers();
    const card = completedBlock(firstEvents, "action_card");

    expect(firstEvents.slice(-2)).toEqual([
      expect.objectContaining({ event: "turn.status", status: "waiting" }),
      expect.objectContaining({
        event: "turn.completed",
        turn_id: firstHandle.turnId,
        status: "blocked",
      }),
    ]);
    const prepared = engine.prepareActionContinuation(
      "conversation",
      card.origin_turn_id,
      [actionReport(card, "completed")],
      (event) => continuationEvents.push(event),
    );
    const continuationHandle = prepared.resume("verified");
    await finishTimers();

    expect(continuationHandle?.turnId).not.toBe(firstHandle.turnId);
    expect(world.connected).toContain("api-github");
    expect(
      continuationEvents
        .filter((event) => event.event === "block.completed")
        .map((event) =>
          event.block.type === "text" ? event.block.text : null,
        ),
    ).toEqual(expect.arrayContaining(["Connected branch", "After await"]));
  });

  it.each([
    ["declined", "Declined branch"],
    ["failed", "Failed branch"],
  ] as const)("selects the %s action branch", async (disposition, expected) => {
    const { engine, world } = createEngine([
      scenario("action", /go/, (script) =>
        script
          .action("service.connect", { service: "api-github" })
          .await({
            declined: (branch) => branch.say("Declined branch"),
            failed: (branch) => branch.say("Failed branch"),
          })
          .connect("api-github"),
      ),
    ]);
    const firstEvents: TurnEvent[] = [];
    const resumed: TurnEvent[] = [];
    engine.play("conversation", matchOrThrow(engine), (event) =>
      firstEvents.push(event),
    );
    await finishTimers();
    const card = completedBlock(firstEvents, "action_card");

    engine
      .prepareActionContinuation(
        "conversation",
        card.origin_turn_id,
        [actionReport(card, disposition)],
        (event) => resumed.push(event),
      )
      .resume("unverified");
    await finishTimers();

    expect(resumed).toContainEqual(
      expect.objectContaining({
        event: "block.completed",
        block: expect.objectContaining({ type: "text", text: expected }),
      }),
    );
    expect(world.connected).not.toContain("api-github");
  });

  it("takes approval approved and denied branches", async () => {
    const definition = scenario("approval", /go/, (script) =>
      script.approval("api-github", "Post digest?").await({
        approved: (branch) => branch.say("Posted"),
        denied: (branch) => branch.say("Skipped"),
      }),
    );

    for (const [approved, expected] of [
      [true, "Posted"],
      [false, "Skipped"],
    ] as const) {
      const { engine } = createEngine([definition]);
      const firstEvents: TurnEvent[] = [];
      const resumed: TurnEvent[] = [];
      engine.play("conversation", matchOrThrow(engine), (event) =>
        firstEvents.push(event),
      );
      await finishTimers();
      const card = completedBlock(firstEvents, "approval_card");

      engine.decideApproval("conversation", card.block_id, approved, (event) =>
        resumed.push(event),
      );
      await finishTimers();

      expect(resumed).toContainEqual(
        expect.objectContaining({
          event: "block.completed",
          block: expect.objectContaining({ type: "text", text: expected }),
        }),
      );
    }
  });

  it("wakes through the default branch and consumes the continuation", async () => {
    const { engine } = createEngine([
      scenario("wake", /go/, (script) =>
        script
          .action("service.connect", { service: "api-github" })
          .await()
          .say("After wake"),
      ),
    ]);
    const parked: TurnEvent[] = [];
    const resumed: TurnEvent[] = [];
    engine.play("conversation", matchOrThrow(engine), (event) =>
      parked.push(event),
    );
    await finishTimers();
    const card = completedBlock(parked, "action_card");

    const handle = engine.wake(
      "conversation",
      card.origin_turn_id,
      (event) => resumed.push(event),
    );
    await finishTimers();

    expect(handle.turnId).not.toBe(card.origin_turn_id);
    expect(engine.hasContinuation("conversation")).toBe(false);
    expect(
      resumed
        .filter(
          (event) =>
            event.event === "block.completed" && event.block.type === "text",
        )
        .map((event) =>
          event.event === "block.completed" && event.block.type === "text"
            ? event.block.text
            : null,
        ),
    ).toEqual([
      "The assistant resumed the blocked turn.",
      "After wake",
    ]);
    expect(() =>
      engine.wake("conversation", card.origin_turn_id),
    ).toThrow("Action continuation was not found.");
  });

  it("splices a need flow only while the world is cold", async () => {
    const flows = {
      connect: flow((script) => script.say("Connecting").connect("api-github")),
    };
    const definition = scenario("need", /go/, (script) =>
      script.need("api-github", "connect").say("Listing"),
    );
    const world = new TestWorld();
    const { engine } = createEngine([definition], flows, { world });
    const coldEvents: TurnEvent[] = [];
    engine.play("cold", matchOrThrow(engine), (event) =>
      coldEvents.push(event),
    );
    await finishTimers();
    expect(world.connected).toContain("api-github");
    expect(
      coldEvents.filter(
        (event) =>
          event.event === "block.completed" && event.block.type === "text",
      ),
    ).toHaveLength(2);

    const warmEvents: TurnEvent[] = [];
    engine.play("warm", matchOrThrow(engine), (event) =>
      warmEvents.push(event),
    );
    await finishTimers();
    expect(
      warmEvents.filter(
        (event) =>
          event.event === "block.completed" && event.block.type === "text",
      ),
    ).toHaveLength(1);
  });

  it("evaluates whenConnected and whenNotConnected at playback", async () => {
    const definition = scenario("condition", /go/, (script) =>
      script
        .whenConnected("api-github", (branch) => branch.say("warm"))
        .whenNotConnected("api-github", (branch) => branch.say("cold")),
    );
    const world = new TestWorld();
    world.connect("api-github");
    const { engine } = createEngine([definition], {}, { world });
    const events: TurnEvent[] = [];

    engine.play("conversation", matchOrThrow(engine), (event) =>
      events.push(event),
    );
    await finishTimers();

    expect(events).toContainEqual(
      expect.objectContaining({
        event: "block.completed",
        block: expect.objectContaining({ type: "text", text: "warm" }),
      }),
    );
    expect(events).not.toContainEqual(
      expect.objectContaining({
        block: expect.objectContaining({ type: "text", text: "cold" }),
      }),
    );
  });
});

describe("ScenarioEngine lifecycle guards", () => {
  it("keeps cursors conversation-monotonic across a continuation (F1)", async () => {
    const { engine } = createEngine(
      [
        scenario("cursor", /go/, (script) =>
          script
            .action("service.connect", { service: "api-github" })
            .await({ completed: (branch) => branch.say("resumed") }),
        ),
      ],
      {},
      { cursors: new TestCursors(20) },
    );
    let state: TurnReducerState = EMPTY_TURN_STATE;
    const delivered: TurnEvent[] = [];
    const apply = (event: TurnEvent) => {
      delivered.push(event);
      state = applyTurnEvent(state, event);
    };
    engine.play("conversation", matchOrThrow(engine), apply);
    await finishTimers();
    const firstTerminalCursor = state.lastCursor;
    const card = completedBlock(delivered, "action_card");

    engine
      .prepareActionContinuation(
        "conversation",
        card.origin_turn_id,
        [actionReport(card, "completed")],
        apply,
      )
      .resume("verified");
    await finishTimers();

    expect(delivered.at(-1)?.cursor).toBeGreaterThan(firstTerminalCursor);
    expect(state.messages).toContainEqual(
      expect.objectContaining({
        blocks: expect.arrayContaining([
          expect.objectContaining({ type: "text", text: "resumed" }),
        ]),
      }),
    );
  });

  it("cancels mid-segment with closed blocks, message, status, then terminal", () => {
    const { engine } = createEngine([
      scenario("cancel", /go/, (script) => script.say("x".repeat(200))),
    ]);
    const events: TurnEvent[] = [];
    const handle = engine.play("conversation", matchOrThrow(engine), (event) =>
      events.push(event),
    );
    vi.advanceTimersByTime(250);

    handle.cancel();

    expect(events.slice(-3)).toEqual([
      expect.objectContaining({ event: "message.completed" }),
      expect.objectContaining({ event: "turn.status", status: "cancelled" }),
      expect.objectContaining({
        event: "turn.completed",
        status: "cancelled",
      }),
    ]);
    const openBlock = events.find((event) => event.event === "block.started");
    expect(events).toContainEqual(
      expect.objectContaining({
        event: "block.completed",
        block_id:
          openBlock?.event === "block.started" ? openBlock.block_id : "missing",
      }),
    );
  });

  it("settles and removes a parked action continuation on cancel", async () => {
    const { engine } = createEngine([
      scenario("parked", /go/, (script) =>
        script
          .action("service.connect", { service: "api-github" })
          .await({ completed: (branch) => branch.say("never") }),
      ),
    ]);
    const events: TurnEvent[] = [];
    engine.play("conversation", matchOrThrow(engine), (event) =>
      events.push(event),
    );
    await finishTimers();

    engine.cancel("conversation");

    expect(engine.hasContinuation("conversation")).toBe(false);
    expect(events.at(-1)).toMatchObject({
      event: "block.updated",
      patch: {
        status: "conflicted",
        outcome_note: "This mock action was cancelled.",
      },
    });
  });

  it("replicates action-card progress and blocked state guards", async () => {
    const { engine } = createEngine([
      scenario("guards", /go/, (script) =>
        script.action("service.connect", { service: "api-github" }).await(),
      ),
    ]);
    const events: TurnEvent[] = [];
    engine.play("conversation", matchOrThrow(engine), (event) =>
      events.push(event),
    );
    await finishTimers();
    const card = completedBlock(events, "action_card");

    engine.setActionCardInProgress(
      "conversation",
      card.block_id,
      true,
      (event) => events.push(event),
    );
    engine.blockActionCard(
      "conversation",
      card.block_id,
      "Dialog closed",
      (event) => events.push(event),
    );
    engine.setActionCardInProgress(
      "conversation",
      card.block_id,
      false,
      (event) => events.push(event),
    );

    expect(events.slice(-2)).toEqual([
      expect.objectContaining({
        event: "block.updated",
        patch: { status: "in_progress", outcome_note: "" },
      }),
      expect.objectContaining({
        event: "block.updated",
        patch: { status: "blocked", outcome_note: "Dialog closed" },
      }),
    ]);
  });

  it("expires approvals lazily only after expires_at (F11)", async () => {
    let now = 1_000;
    const definition = scenario("expiry", /go/, (script) =>
      script
        .approval("api-github", "Post?")
        .await({ approved: (branch) => branch.say("approved") }),
    );

    const before = createEngine([definition], {}, { now: () => now }).engine;
    const beforeEvents: TurnEvent[] = [];
    before.play("before", matchOrThrow(before), (event) =>
      beforeEvents.push(event),
    );
    await finishTimers();
    const beforeCard = completedBlock(beforeEvents, "approval_card");
    now = Date.parse(beforeCard.expires_at) - 1;
    expect(
      before.decideApproval("before", beforeCard.block_id, true, (event) =>
        beforeEvents.push(event),
      ),
    ).not.toBeNull();

    now = 2_000;
    const after = createEngine([definition], {}, { now: () => now }).engine;
    const afterEvents: TurnEvent[] = [];
    after.play("after", matchOrThrow(after), (event) =>
      afterEvents.push(event),
    );
    await finishTimers();
    const afterCard = completedBlock(afterEvents, "approval_card");
    now = Date.parse(afterCard.expires_at) + 1;

    expect(
      after.decideApproval("after", afterCard.block_id, true, (event) =>
        afterEvents.push(event),
      ),
    ).toBeNull();
    expect(after.hasContinuation("after")).toBe(false);
    expect(afterEvents.at(-1)).toMatchObject({
      event: "block.updated",
      patch: { decision: "expired" },
    });
  });
});

beforeEach(() => {
  vi.useFakeTimers();
});

afterEach(() => {
  vi.useRealTimers();
});
