import { afterEach, expect, it, vi } from "vitest";

const key = "nyxid.assistant.wirelog.v1";
const exchange = {
  id: "fixture-exchange",
  ts: 1794900000000,
  kind: "header",
  status: 200,
  conversationId: "fixture-chat",
  wireLogId: "fixture-wire",
  label: "POST /assistant/chat",
};
afterEach(() => {
  vi.unstubAllGlobals();
});
it.each(["A", "B", undefined])(
  "cold auth binds persisted v3 wire history owned by %s",
  async (ownerUserId) => {
    vi.resetModules();
    localStorage.clear();
    localStorage.setItem(
      key,
      JSON.stringify({
        version: 3,
        state: {
          ownerUserId,
          entries: [exchange],
          captureEnabled: true,
          showResponses: false,
        },
      }),
    );
    const { useAssistantWireLogStore: wire } =
      await import("./assistant-wire-log-store");
    const { useAuthStore: auth } = await import("./auth-store");
    vi.stubGlobal(
      "fetch",
      vi
        .fn()
        .mockResolvedValue(Response.json({ id: "A", email: "a@example.test" })),
    );
    expect(await auth.getState().checkAuth()).toBe(true);
    const same = ownerUserId === "A";
    expect(wire.getState().entries).toEqual(same ? [exchange] : []);
    expect(wire.getState().captureEnabled).toBe(same);
    expect(wire.getState().showResponses).toBe(!same);
    const raw = localStorage.getItem(`${key}:A`) ?? localStorage.getItem(key);
    if (same) {
      expect(raw).not.toBeNull();
      expect(JSON.parse(raw!).state).toMatchObject({
        ownerUserId: "A",
        entries: [exchange],
        captureEnabled: true,
        showResponses: false,
      });
    } else {
      expect(raw ?? "").not.toContain("fixture-exchange");
      expect(raw ?? "").not.toContain('"captureEnabled":true');
    }
  },
);
