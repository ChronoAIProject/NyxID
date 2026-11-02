import { expect, it } from "vitest";
import {
  remoteMachineCreateRequestSchema,
  remoteMachineSchema,
} from "./machines";
import {
  getRemoteMachineIntent,
  readRemoteMachineIntent,
} from "@/lib/remote-machine-intent";
import { remoteMachineFixture } from "@/test/remote-machine-fixture";

const create = {
  name: "research",
  capabilities: ["shell", "files", "shell"],
  specialist_id: null,
  profile_version: "remote-machine",
  free_slot: true,
  policy_version: "remote-machine-mvp",
  data_policy_version: "remote-machine-data",
  acknowledge_shared_environment: true,
  separation_choice: "automatic",
  separation_policy_version: "remote-machine-separation-v1",
  acknowledge_separation: true,
};

it.each(["automatic", "require_separated", "shared_legacy"])(
  "accepts explicit %s consent and canonicalizes the full intent",
  (separation_choice) => {
    sessionStorage.clear();
    const body = remoteMachineCreateRequestSchema.parse({
      ...create,
      separation_choice,
    });
    expect(body.capabilities).toEqual(["files", "shell"]);
    const first = getRemoteMachineIntent("owner", "create", body);
    expect(first.body).toEqual(body);
    expect(readRemoteMachineIntent("owner", "create")).toEqual(first);
    const retry = getRemoteMachineIntent("owner", "create", {
      ...body,
      separation_choice: "shared_legacy",
    });
    expect(retry).toEqual(first);
  },
);

it.each([
  { separation_choice: undefined },
  { separation_choice: "separated" },
  { separation_policy_version: undefined },
  { separation_policy_version: "unreviewed" },
  { acknowledge_separation: undefined },
  { acknowledge_separation: false },
])("refuses incomplete or unreviewed hosted separation consent %j", (patch) => {
  expect(
    remoteMachineCreateRequestSchema.safeParse({ ...create, ...patch }).success,
  ).toBe(false);
});

it("refuses incomplete stored create intent without synthesizing a shared choice", () => {
  const key = "nyxid:remote-machine-intent:owner:create";
  const raw = JSON.stringify({
    idempotencyKey: crypto.randomUUID(),
    body: { ...create, separation_choice: undefined },
  });
  sessionStorage.setItem(key, raw);
  expect(() => readRemoteMachineIntent("owner", "create")).toThrow(/reset/);
  expect(sessionStorage.getItem(key)).toBe(raw);
  sessionStorage.clear();
});

it("keeps target hosted configuration and its current evidence result independent of lifecycle revision", () => {
  const machine_configuration = {
    version: 2,
    revision: 7,
    digest: "a".repeat(64),
    runtime_generation: 4,
    state: "pending",
    reason: "current_runtime_configuration_evidence_required",
  };
  const parsed = remoteMachineSchema.parse({
    ...remoteMachineFixture,
    revision: 51,
    runtime_generation: 4,
    separation_choice: "require_separated",
    separation_policy_version: "remote-machine-separation-v1",
    machine_configuration,
  });
  expect(parsed).toMatchObject({ revision: 51, machine_configuration });
  expect(
    remoteMachineSchema.safeParse({
      ...parsed,
      machine_configuration: undefined,
    }).success,
  ).toBe(false);
});
