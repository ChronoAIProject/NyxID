import { test } from "node:test";
import assert from "node:assert/strict";
import { runFaceContracts } from "../../../../shared/agent-avatars/contract-cases";
test("native shared face contract: reference, persistence, topology and lifecycle", () =>
  runFaceContracts((ok, message) => assert.ok(ok, message)));
