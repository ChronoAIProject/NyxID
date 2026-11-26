import { it, expect } from "vitest";
import { runFaceContracts } from "../../../../shared/agent-avatars/contract-cases";
it("satisfies reference, persistence, appearance, topology and lifecycle contracts", () =>
  runFaceContracts((ok, message) => expect(ok, message).toBeTruthy()));
