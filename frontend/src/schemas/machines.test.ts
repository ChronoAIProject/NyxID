import { describe, expect, it } from "vitest";
import {
  remoteMachineFixture,
  remoteMachinePlatformFixture,
} from "@/test/remote-machine-fixture";
import {
  machineSetupCommand,
  remoteMachineSchema,
  remoteMachineCreateRequestSchema,
  remoteMachineActionRequestSchema,
  remoteMachineQuoteSchema,
  remoteMachineQuoteRequestSchema,
  remoteMachineRuntimeBillingSchema,
  remoteMachineBillingRunsSchema,
  isRemoteMachinePaidQuote,
  remoteMachinePaidQuoteSchema,
  remoteMachineEntitlementSchema,
  remoteMachineJobsSchema,
  remoteMachineConfirmationSchema,
  remoteMachineJobOutputSchema,
} from "./machines";

const time = "2026-10-14T01:00:00Z";
const paidQuote = {
  platform: remoteMachinePlatformFixture,
  policy_version: "remote-machine-mvp",
  profile: remoteMachineFixture.profile,
  free_slot_requested: false,
  paid_admission: "quote_acceptance_required",
  pricing_status: "synced",
  valid_until: "2026-10-14T01:15:00Z",
  quote_id: "quote-one",
  rate_version: "rate-one",
  credits_per_second: "0.000040000001",
  funding_mode: "account_balance",
  no_instance_cap: true,
  cumulative_whole_seconds: true,
  paused_storage_free: true,
  policy: {
    policy_version: "remote-machine-mvp",
    data_policy_version: "remote-machine-data",
    free_slot_per_personal_account: 1,
    free_entitlement_expires: false,
    free_monthly_runtime_limit_seconds: null,
    paused_workspace_retention: "indefinite_until_owner_confirmed_delete",
    paused_storage_price_credits: "0",
    workspace_backup_or_restore: false,
    owner_confirmed_delete_required: true,
    cleanup_target_hours: 24,
    job_history_days: 30,
    operation_history_days: 90,
    terminal_output_max_retention_seconds: 3600,
    shared_environment_mode: "shared_legacy",
    cross_client_secret_isolation: false,
    inbound_network_service: false,
    persistent_environment_variables: false,
    service_charges_separate: true,
    concurrent_jobs: 2,
    default_command_timeout_seconds: 120,
    maximum_command_timeout_seconds: 3600,
    job_output_bytes: 1048576,
    machine_output_cache_bytes: 33554432,
  },
};

describe("Remote Machine safe schemas", () => {
  it.each([null, undefined])("accepts an undeclared current temporary capacity: %s", (ephemeral) => {
    const profile = { ...remoteMachineFixture.profile, ephemeral_gib: ephemeral };
    if (ephemeral === undefined) delete profile.ephemeral_gib;
    const machine = remoteMachineSchema.parse({ ...remoteMachineFixture, profile });
    expect(machine.profile.ephemeral_gib).toBe(ephemeral);
    expect(machine.profile.memory_mib).toBe(2048);
    expect(machine.profile.workspace_gib).toBe(20);
  });

  it("rejects obsolete contracts and altered fixed resources", () => {
    for (const patch of [
      { policy_version: "remote-machine-mvp-v1" }, { policy_version: "remote-machine-mvp-v2" },
      { data_policy_version: "remote-machine-data-v1" },
      ...["remote-shell-files-v1", "remote-shell-files-v2"].map(profile_version => ({ profile: { ...remoteMachineFixture.profile, profile_version } })),
      ...[{ ephemeral_gib: 0 }, { ephemeral_gib: 2 }, { cpu_millis: 4000 }, { memory_mib: 4096 }, { workspace_gib: 40 }].map(profile => ({ profile: { ...remoteMachineFixture.profile, ...profile } })),
    ]) expect(remoteMachineSchema.safeParse({ ...remoteMachineFixture, ...patch }).success).toBe(false);
  });

  it("accepts actual paid offer fields only before their expiry", () => {
    const parsed = remoteMachinePaidQuoteSchema.parse({
      ...paidQuote,
      platform: {
        configuration_ready: true,
        manager_started: true,
        namespace: "nyxid-machine",
        blockers: [],
        physical_acceptance: "native_state_only_no_physical_guarantee",
        lifecycle_basis: "agent_sandbox_native_observation",
      },
    });
    expect(parsed.credits_per_second).toBe("0.000040000001");
    expect(isRemoteMachinePaidQuote(paidQuote, Date.parse(time))).toBe(true);
    expect(
      isRemoteMachinePaidQuote(paidQuote, Date.parse(paidQuote.valid_until)),
    ).toBe(false);
    expect(
      isRemoteMachinePaidQuote(
        paidQuote,
        Date.parse(paidQuote.valid_until) + 1,
      ),
    ).toBe(false);
  });
  it("preserves platform readiness and never treats configuration as physical acceptance", () => {
    const platform = {
      configuration_ready: false,
      manager_started: false,
      namespace: "nyxid-machine",
      blockers: ["storage_class_missing"],
      physical_acceptance: "native_state_only_no_physical_guarantee",
      lifecycle_basis: "agent_sandbox_native_observation",
    };
    expect(
      remoteMachineEntitlementSchema.parse({
        eligible: true,
        free_slot_available: true,
        machine_id: null,
        reason: null,
        policy_version: "remote-machine-mvp",
        policy: paidQuote.policy,
        platform,
      }).platform,
    ).toEqual(platform);
    expect(
      remoteMachinePaidQuoteSchema.safeParse({ ...paidQuote, platform })
        .success,
    ).toBe(true);
  });
  it("rejects unsynchronized, free, zero-rate and incomplete paid offers", () => {
    for (const patch of [
      { pricing_status: "pending" },
      { paid_admission: "disabled_until_p3" },
      { free_slot_requested: true },
      { credits_per_second: "0.000000000000" },
      { credits_per_second: "-0.00004" },
      { credits_per_second: 0.00004 },
      { quote_id: null },
      { rate_version: null },
      { no_instance_cap: false },
      { cumulative_whole_seconds: false },
      { paused_storage_free: false },
      { funding_mode: "overdraft" },
    ])
      expect(
        isRemoteMachinePaidQuote({ ...paidQuote, ...patch }, Date.parse(time)),
      ).toBe(false);
  });
  it("keeps additive runtime funding fields without inferring them for older servers", () => {
    expect(
      remoteMachineSchema.parse(remoteMachineFixture).free_slot,
    ).toBeUndefined();
    const parsed = remoteMachineSchema.parse({
      ...remoteMachineFixture,
      free_slot: false,
      runtime_run_id: "run-one",
      funded_until: time,
    });
    expect(parsed.free_slot).toBe(false);
    expect(parsed.runtime_run_id).toBe("run-one");
    expect(parsed.funded_until).toBe(time);
  });
  it("requires the quote and charging acceptance together for paid create", () => {
    const body = {
      free_slot: false,
      policy_version: "remote-machine-mvp",
      data_policy_version: "remote-machine-data",
      acknowledge_shared_environment: true,
    };
    expect(remoteMachineCreateRequestSchema.safeParse(body).success).toBe(
      false,
    );
    expect(
      remoteMachineCreateRequestSchema.safeParse({
        ...body,
        quote_id: "quote-one",
      }).success,
    ).toBe(false);
    expect(
      remoteMachineCreateRequestSchema.safeParse({
        ...body,
        accept_account_balance: true,
      }).success,
    ).toBe(false);
    expect(
      remoteMachineCreateRequestSchema.safeParse({
        ...body,
        quote_id: "quote-one",
        accept_account_balance: true,
      }).success,
    ).toBe(true);
    expect(
      remoteMachineCreateRequestSchema.safeParse({
        ...body,
        free_slot: true,
        quote_id: "quote-one",
        accept_account_balance: true,
      }).success,
    ).toBe(false);
  });
  it("permits a paired paid resume acceptance without attaching charges to other actions", () => {
    const body = {
      action: "resume",
      expected_revision: 4,
      policy_version: "remote-machine-mvp",
      data_policy_version: "remote-machine-data",
      acknowledge_shared_environment: true,
      quote_id: "quote-one",
      accept_account_balance: true,
    };
    expect(remoteMachineActionRequestSchema.safeParse(body).success).toBe(true);
    expect(
      remoteMachineActionRequestSchema.safeParse({
        ...body,
        accept_account_balance: false,
      }).success,
    ).toBe(false);
    expect(
      remoteMachineActionRequestSchema.safeParse({ ...body, action: "pause" })
        .success,
    ).toBe(false);
  });
  it("keeps observed lifecycle states separate and strips non-view fields", () => {
    const result = remoteMachineSchema.parse({
      ...remoteMachineFixture,
      token: "never-cache-this",
      bootstrap_material: "never-cache-this-either",
      profile: { ...remoteMachineFixture.profile, image_secret: "secret" },
    });
    expect(result.resource_state).toBe("provisioning");
    expect(result.billing_state).toBe("free");
    expect(result.authorization_state).toBe("pending");
    expect(result.last_heartbeat_at).toBeNull();
    expect(result).not.toHaveProperty("token");
    expect(result).not.toHaveProperty("bootstrap_material");
    expect(result.profile).not.toHaveProperty("image_secret");
    expect(result).not.toHaveProperty("disk_used_bytes");
  });

  it("requires explicit policy acceptance for creation", () => {
    const body = {
      free_slot: true,
      policy_version: "remote-machine-mvp",
      data_policy_version: "remote-machine-data",
      acknowledge_shared_environment: true,
    };
    expect(remoteMachineCreateRequestSchema.safeParse(body).success).toBe(true);
    expect(
      remoteMachineCreateRequestSchema.safeParse({
        ...body,
        acknowledge_shared_environment: false,
      }).success,
    ).toBe(false);
    expect(
      remoteMachineCreateRequestSchema.safeParse({
        ...body,
        image: "caller-image",
      }).success,
    ).toBe(false);
  });

  it("requires a current revision and irreversible data-loss acknowledgement to delete", () => {
    const body = { action: "delete", expected_revision: 2 };
    expect(remoteMachineActionRequestSchema.safeParse(body).success).toBe(
      false,
    );
    expect(
      remoteMachineActionRequestSchema.safeParse({
        ...body,
        data_policy_version: "remote-machine-data",
        acknowledge_data_loss: true,
      }).success,
    ).toBe(true);
    expect(
      remoteMachineActionRequestSchema.safeParse({ action: "pause" }).success,
    ).toBe(false);
  });

  it("does not accept resume as an automatic top-up effect", () => {
    const body = { action: "resume", expected_revision: 2 };
    expect(remoteMachineActionRequestSchema.safeParse(body).success).toBe(
      false,
    );
    expect(
      remoteMachineActionRequestSchema.safeParse({
        ...body,
        policy_version: "remote-machine-mvp",
        data_policy_version: "remote-machine-data",
        acknowledge_shared_environment: true,
      }).success,
    ).toBe(true);
  });

  it("does not mistake the current unpriced profile quote for an accepted paid quote", () => {
    expect(
      remoteMachineQuoteSchema.safeParse({
        quote_id: "unverified",
        accepted_credits_per_second: "0.00004",
        ready: true,
      }).success,
    ).toBe(false);
  });

  it("preserves exact runtime money and does not add service costs into runtime totals", () => {
    const detail = {
      node_id: "machine-1",
      run_id: "run-1",
      state: "stop_pending",
      rate_version: "rate-1",
      accepted_credits_per_second: "0.000040",
      covered_until: time,
      cumulative_seconds: 23,
      settled_credits: "0.000920000001",
      reserved_credits: "0.012",
      pending_credits: "0.012",
      funds: {
        wallet_available: "3.000000000001",
        wallet_reserved: "0.012",
        pending_debits: "0",
        pending_expiry: "0",
        matching_allowance_seconds: 0,
        available_grants: "0",
        estimated_seconds: "75000",
        shared_account_estimate_only: true,
      },
      intervals: [],
      next_interval_offset: null,
      attribution: "machine_runtime",
      service_costs_included: false,
    };
    const parsed = remoteMachineRuntimeBillingSchema.parse(detail);
    expect(parsed.settled_credits).toBe("0.000920000001");
    expect(parsed.pending_credits).toBe(parsed.reserved_credits);
    expect(
      remoteMachineRuntimeBillingSchema.safeParse({
        ...detail,
        settled_credits: 0.00092,
      }).success,
    ).toBe(false);
    expect(
      remoteMachineRuntimeBillingSchema.safeParse({
        ...detail,
        service_costs_included: true,
      }).success,
    ).toBe(false);
  });

  it("keeps historical run versions separate without fabricating a close timestamp", () => {
    const history = remoteMachineBillingRunsSchema.parse({
      runs: [
        {
          run_id: "old-run",
          rate_version: "old-rate",
          accepted_credits_per_second: "0.000040",
          state: "closed",
          created_at: time,
          updated_at: time,
        },
        {
          run_id: "new-run",
          rate_version: "new-rate",
          accepted_credits_per_second: "0.000050000001",
          state: "running",
          created_at: time,
          updated_at: time,
        },
      ],
      next_offset: 25,
    });
    expect(history.runs.map((run) => run.accepted_credits_per_second)).toEqual([
      "0.000040",
      "0.000050000001",
    ]);
    expect(history.runs[0]).not.toHaveProperty("closed_at");
    expect(history.next_cursor).toBe("25");
  });
  it("accepts current owner history fields without fabricating execution details", () => {
    const parsed = remoteMachineJobsSchema.parse({
      jobs: [
        {
          id: "job-one",
          node_id: "machine-1",
          runtime_generation: null,
          status: "finished",
          created_at: time,
          started_at: null,
          finished_at: time,
          expires_at: time,
          exit_code: 0,
          stop_reason: "completed",
          duration_ms: 123,
          cancellation_requested_by: null,
          cancellation_requested_at: null,
          retain_until: "2026-11-13T01:00:00Z",
          output_availability: "query_required",
          output_query_available: true,
          output_expires_at: null,
          output_reason: null,
          services: [],
          subject: {
            kind: "assistant",
            user_id: "owner-one",
            agent_id: null,
            conversation_id: null,
          },
          grant_id: null,
          cancellation_available: false,
        },
      ],
      next_cursor: null,
    });
    expect(parsed.jobs[0]?.runtime_generation).toBeNull();
    expect(parsed.jobs[0]?.output_query_available).toBe(true);
  });
  it("accepts a minimal live output response returned by the Node", () => {
    expect(
      remoteMachineJobOutputSchema.parse({
        job_id: "job-one",
        status: "running",
        stdout: "hello",
      }),
    ).toMatchObject({ job_id: "job-one", stdout: "hello" });
  });
  it("preserves server safe confirmation summaries", () => {
    expect(
      remoteMachineConfirmationSchema.parse({
        id: "confirmation-one",
        node_id: "machine-1",
        subject: {
          kind: "oauth",
          user_id: "owner-one",
          oauth_client_id: "client-one",
        },
        grant_id: "grant-one",
        grant_revision: 2,
        assignment_revision: 3,
        runtime_id: "runtime-one",
        runtime_generation: 4,
        operation: "shell.exec",
        parameters_digest: "digest",
        safe_parameters: [{ name: "cwd", value: "/workspace" }],
        review_parameters_available: true,
        review_content_available: false,
        status: "pending",
        created_at: time,
        expires_at: "2026-10-14T01:15:00Z",
        approved_at: null,
        approved_until: null,
      }).safe_parameters,
    ).toEqual([{ name: "cwd", value: "/workspace" }]);
  });
});

const hostedCreate = {
  free_slot: true,
  policy_version: "remote-machine-mvp",
  data_policy_version: "remote-machine-data",
  acknowledge_shared_environment: true,
};
it("defaults hosted creation and preserves explicit configuration in the safe projection", () => {
  expect(remoteMachineCreateRequestSchema.parse(hostedCreate)).toMatchObject({
    name: "my-remote-machine",
    capabilities: ["shell", "files"],
    specialist_id: null,
  });
  const config = {
    name: "research",
    capabilities: ["browser", "computer"],
    specialist_id: "specialist-1",
  };
  expect(
    remoteMachineCreateRequestSchema.parse({ ...hostedCreate, ...config }),
  ).toMatchObject(config);
  expect(
    remoteMachineSchema.parse({
      ...remoteMachineFixture,
      ...config,
      specialist_assignment_state: "permissions_required",
      specialist_assignment_error: "specialist_assignment_unavailable",
    }),
  ).toMatchObject({
    ...config,
    specialist_assignment_state: "permissions_required",
    specialist_assignment_error: "specialist_assignment_unavailable",
  });
});
it("rejects invalid hosted names, empty or unknown capabilities and desktop without browser", () => {
  for (const patch of [
    { name: "" },
    { name: "Uppercase" },
    { name: "x".repeat(65) },
    { capabilities: [] },
    { capabilities: ["ssh"] },
    { capabilities: ["computer"] },
    { owner_id: "org" },
  ]) {
    expect(
      remoteMachineCreateRequestSchema.safeParse({ ...hostedCreate, ...patch })
        .success,
    ).toBe(false);
  }
});


it("binds request schemas to the sole controlled profile and rejects arbitrary dimensions", () => {
  const body = {
    free_slot: true,
    profile_version: "remote-machine",
    policy_version: "remote-machine-mvp",
    data_policy_version: "remote-machine-data",
    acknowledge_shared_environment: true,
  };
  expect(remoteMachineCreateRequestSchema.parse(body).profile_version).toBe(
    "remote-machine",
  );
  expect(
    remoteMachineQuoteRequestSchema.parse({
      free_slot: false,
      profile_version: "remote-machine",
    }).profile_version,
  ).toBe("remote-machine");
  for (const patch of [
    { profile_version: "foreign-profile" },
    { profile_version: null },
    { cpu_millis: 4000 },
    { workspace_gib: 50 },
  ]) {
    expect(
      remoteMachineCreateRequestSchema.safeParse({ ...body, ...patch }).success,
    ).toBe(false);
    expect(
      remoteMachineQuoteRequestSchema.safeParse({
        free_slot: false,
        profile_version: "remote-machine",
        ...patch,
      }).success,
    ).toBe(false);
  }
});

it("requires explicit legacy acknowledgement and carries selection to native and Docker setup", async () => {
  const { machineChoicesSchema, machineSetupCommand } = await import("./machines");
  const choices = { name: "machine", where: "vm" as const, capabilities: ["shell" as const], grant_to: null };
  const token = `nyx_nreg_${"a".repeat(64)}`;
  expect(machineChoicesSchema.safeParse({ ...choices, separation:null, legacy_warning_acknowledged:false }).success).toBe(true);
  expect(machineSetupCommand(choices, token, "wss://nyxid.example/api/v1/nodes/ws")).toContain("node setup --separation 'automatic'");
  expect(machineChoicesSchema.safeParse({ ...choices, separation: "shared_legacy" }).success).toBe(false);
  expect(() => machineSetupCommand({ ...choices, separation: "shared_legacy" }, token, "wss://nyxid.example/api/v1/nodes/ws")).toThrow(/acknowledge/);
  const explicit = { ...choices, separation: "shared_legacy" as const, legacy_warning_acknowledged: true };
  expect(machineSetupCommand(explicit, token, "wss://nyxid.example/api/v1/nodes/ws")).toContain("--separation 'shared-legacy'");
  const image = `ghcr.io/chronoaiproject/nyxid/nyxid-machine-updater@sha256:${"ab".repeat(32)}`;
  expect(machineSetupCommand({ ...explicit, where: "docker" }, token, "wss://nyxid.example/api/v1/nodes/ws", "0.72.0", image)).toContain("NYXID_MACHINE_SEPARATION=shared-legacy");
  expect(machineSetupCommand({ ...choices, where: "docker", separation: "require_separated" }, token, "wss://nyxid.example/api/v1/nodes/ws", "0.72.0", image)).toContain("NYXID_MACHINE_SEPARATION=require-separated");
});


it("keeps native setup limited to the selected capabilities", () => {
  const token = `nyx_nreg_${"a".repeat(64)}`;
  for (const capability of ["shell", "files", "browser", "computer"] as const) {
    const command = machineSetupCommand({
      name: "limited-machine", where: "this_computer", grant_to: null,
      separation: "shared_legacy", legacy_warning_acknowledged: true,
      capabilities: [capability],
    }, token, "wss://nyxid.example/api/v1/nodes/ws");
    expect(command).toContain(` --${capability}`);
    expect(command).not.toContain(" --machine");
    for (const excluded of ["shell", "files", "browser", "computer"].filter(value => value !== capability)) {
      expect(command).not.toContain(` --${excluded}`);
    }
  }
});
