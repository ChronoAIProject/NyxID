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
  expect(
    parseMachineDesktopSearch({
      context_id: "12345678-1234-4234-8234-123456789abc",
    }).context_id,
  ).toBe("12345678-1234-4234-8234-123456789abc");
  expect(
    parseMachineDesktopSearch({ context_id: "ctx_secret!" }).context_id,
  ).toBeUndefined();
  expect(
    parseMachinesSearch({
      machine: "machine-1",
      operation: "operation-1",
      confirmation: "confirmation-1",
      token: "not-allowed",
    }),
  ).toEqual({
    tab: undefined,
    machine: "machine-1",
    operation: "operation-1",
    confirmation: "confirmation-1",
  });
  expect(
    parseMachinesSearch({
      operation: "x".repeat(129),
      confirmation: "../secret?token=x",
    }),
  ).toEqual({ tab: undefined });
});

it("allows the hosted setup shortcut without propagating an arbitrary mode", () => {
  expect(parseMachineSetupSearch({ where: "hosted" })).toMatchObject({
    where: "hosted",
  });
  expect(parseMachineSetupSearch({ where: "unknown" })).not.toHaveProperty(
    "where",
    "unknown",
  );
});
