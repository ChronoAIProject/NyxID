import { beforeEach, expect, it } from "vitest";
import { transitionAssistantIdentity } from "@/lib/assistant/identity";
import { createAssistantDraftStore } from "./assistant-draft-store";
import { createAssistantPinsStore } from "./assistant-pins-store";

beforeEach(() => localStorage.clear());
it("a resumed A tab cannot delete B's draft saved by a tab that closed", async () => {
  transitionAssistantIdentity("A");
  const resumed = createAssistantDraftStore();
  resumed.getState().saveDraft("A", "conv:A", "A private text");
  transitionAssistantIdentity("B");
  const second = createAssistantDraftStore();
  second.getState().saveDraft("B", "conv:B", "B original text");
  // The old tab only has A in memory when its profile confirms B.
  expect(resumed.getState().ownerUserId).toBe("A");
  resumed.getState().clear();
  await resumed.persist.rehydrate();
  const cold = createAssistantDraftStore();
  await cold.persist.rehydrate();
  expect(cold.getState().getDraft("conv:B")).toBe("B original text");
  expect(cold.getState().getDraft("conv:A")).toBe("");
  expect(resumed.getState().getDraft("conv:A")).toBe("");
});
it("a resumed A tab cannot remove B's pins saved by a tab that closed", () => {
  const resumed = createAssistantPinsStore();
  resumed.getState().bindAccount("A");
  resumed.getState().toggle("A", "A private chat");
  const second = createAssistantPinsStore();
  second.getState().bindAccount("B");
  second.getState().toggle("B", "B original chat");
  resumed.getState().bindAccount("B");
  const cold = createAssistantPinsStore();
  cold.getState().bindAccount("B");
  expect(cold.getState().ids).toEqual(["B original chat"]);
  expect(resumed.getState().ids).toEqual(["B original chat"]);
  expect(cold.getState().ids).not.toContain("A private chat");
});
