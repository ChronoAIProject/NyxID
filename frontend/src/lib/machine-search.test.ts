import { expect, it } from "vitest";
import {
  parseMachinesSearch,
  parseMachineSetupSearch,
  parseMachinePairSearch,
  parseMachineDesktopSearch,
} from "./machine-search";
it("validates workspace queries without propagating unrelated fields", () => {
  expect(parseMachinesSearch({ tab: "logins", token: "secret" })).toEqual({
    tab: "logins",
  });
  expect(parseMachinesSearch({ tab: "unknown" }).tab).toBeUndefined();
  expect(parseMachineSetupSearch({ setup: "bad" }).setup).toBeUndefined();
  expect(parseMachinePairSearch({ code: "ABCD-EFGH" }).code).toBe("ABCD-EFGH");
  expect(parseMachinePairSearch({ code: ["ABCD"] }).code).toBeUndefined();
  expect(
    parseMachineDesktopSearch({ conversation_id: "x".repeat(129) })
      .conversation_id,
  ).toBeUndefined();
});
