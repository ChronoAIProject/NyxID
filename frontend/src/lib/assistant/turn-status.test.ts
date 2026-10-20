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
});
