import assert from "node:assert/strict";
import { test } from "node:test";
import {
  runBadgeContracts,
  runQuietMotionContracts,
  runRosterActivityContracts,
  runTurnActivityContracts,
} from "../../../../shared/agent-avatars/activity-contract-cases";

test("resolves shared turn activity facts", () =>
  runTurnActivityContracts(assert.ok));
test("aggregates roster activity across conversations", () =>
  runRosterActivityContracts(assert.ok));
test("distinguishes static activity badges", () =>
  runBadgeContracts(assert.ok));
test("draws distinct quiet motion for every live state", () =>
  runQuietMotionContracts(assert.ok));
