import { describe, expect, it } from "vitest";
import { deriveTurnStatus } from "./turn-status";

const activeTurn = { activities: [], continuations: 0 };
const status = (overrides: Partial<Parameters<typeof deriveTurnStatus>[0]> = {}) =>
  deriveTurnStatus({ activeTurn, live: false, pendingApproval: false, retrying: false, ...overrides })!;

describe("deriveTurnStatus", () => {
  it("uses the persisted phases in precedence order", () => {
    expect(status({ pendingApproval: true, retrying: true }).phase).toBe("approval");
    expect(status({ pendingApproval: false, activeTurn: { activities: [{ status: "running", label: "tool", started_at: "now" }], continuations: 1 }, retrying: true }).phase).toBe("tool");
    expect(status({ retrying: true }).phase).toBe("retrying");
    expect(status({ activeTurn: { activities: [], continuations: 1 } }).phase).toBe("continuing");
    expect(status({ live: true }).phase).toBe("working");
    expect(status().phase).toBe("active");
  });

  it("keeps status visible when content already exists and marks interruption", () => {
    expect(status({ live: false, interrupted: true })).toMatchObject({
      phase: "active",
      interrupted: true,
    });
  });

  it("returns no status after settlement", () => {
    expect(deriveTurnStatus({ activeTurn: null, live: false, pendingApproval: false, retrying: false })).toBeNull();
  });

  it("keeps one turn start across phase changes and uses the oldest running call", () => {
    const started_at = "2026-10-21T00:00:00Z";
    const activeTurn = { started_at, activities: [
      { status: "completed", label: "finished", started_at },
      { status: "running", label: "first", started_at: "2026-10-21T00:00:05Z" },
      { status: "running", label: "second", started_at: "2026-10-21T00:00:10Z" },
    ] };
    expect(status({ activeTurn })).toMatchObject({ startedAt: started_at, toolLabel: "first", toolStartedAt: "2026-10-21T00:00:05Z" });
    expect(status({ activeTurn, pendingApproval: true }).startedAt).toBe(started_at);
    expect(status({ activeTurn: { started_at, activities: [] }, retrying: true }).startedAt).toBe(started_at);
    expect(status({ activeTurn: { started_at, continuations: 1 } }).startedAt).toBe(started_at);
  });
});
