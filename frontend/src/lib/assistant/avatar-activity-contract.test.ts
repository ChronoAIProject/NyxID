import { expect, it } from "vitest";
import {
  runBadgeContracts,
  runQuietMotionContracts,
  runRosterActivityContracts,
  runTurnActivityContracts,
} from "../../../../shared/agent-avatars/activity-contract-cases";

const check = (ok: unknown, message: string) =>
  expect(ok, message).toBeTruthy();

it("resolves shared turn activity facts", () =>
  runTurnActivityContracts(check));
it("aggregates roster activity across conversations", () =>
  runRosterActivityContracts(check));
it("distinguishes static activity badges", () => runBadgeContracts(check));
it("draws distinct quiet motion for every live state", () =>
  runQuietMotionContracts(check));
