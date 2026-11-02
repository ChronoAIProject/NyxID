import { expect, test, type Page } from "@playwright/test";
import { mockDashboard } from "./managed-onboarding-fixtures";
import { billingWallet } from "../src/test/billing-fixture";
import type {
  RemoteMachine,
  RemoteMachineOperation,
} from "../src/schemas/machines";
import {
  remoteMachineFixture,
  remoteMachinePlatformFixture,
  remoteMachineOperationFixture,
} from "../src/test/remote-machine-fixture";

const policy = {
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
  separation_policy_version: "remote-machine-separation-v1",
  separation_choices: ["automatic", "require_separated", "shared_legacy"],
  separation_warning:
    "Shared OS access can expose files through the execution identity.",
  shared_environment_mode: "per_agent",
  cross_client_secret_isolation: false,
  inbound_network_service: false,
  persistent_environment_variables: false,
  service_charges_separate: true,
  concurrent_jobs: 2,
  default_command_timeout_seconds: 120,
  maximum_command_timeout_seconds: 3600,
  job_output_bytes: 1048576,
  machine_output_cache_bytes: 33554432,
};
const machine = {
  ...remoteMachineFixture,
  resource_state: "paused",
  active_operation_id: null,
};

async function machineApi(
  page: Page,
  initiallyEmpty = false,
  configuration: Partial<RemoteMachine> = {},
  featureMode: "absent" | "off" | "on" = "on",
) {
  const writes: { body: unknown; key: string | undefined }[] = [];
  let created = !initiallyEmpty;
  let projection = { ...machine, ...configuration };
  let currentOperation = remoteMachineOperationFixture;
  await page.routeWebSocket("**/*", (socket) => {
    if (new URL(socket.url()).pathname === "/")
      socket.send(JSON.stringify({ type: "connected" }));
    socket.onMessage(() => {});
  });
  // No request reaches a real backend, including unspecified dashboard APIs.
  await page.route("**/*", (route) => {
    const request = route.request();
    const url = new URL(request.url());
    if (
      url.origin === "http://localhost:4627" &&
      request.method() === "GET" &&
      ["document", "script", "stylesheet", "image", "font", "other"].includes(
        request.resourceType(),
      ) &&
      !/^\/(api|oauth|mcp|health|\.well-known)(\/|$)/.test(url.pathname)
    )
      return route.continue();
    return route.abort();
  });
  await mockDashboard(page);
  await page.route("**/api/v1/**", (route) => {
    const request = route.request();
    const path = new URL(request.url()).pathname.replace("/api/v1", "");
    if (
      request.method() === "GET" &&
      ["/orgs", "/api-keys", "/nodes", "/runtime-config"].includes(path)
    )
      return route.fallback();
    return route.fulfill({
      status: 404,
      json: {
        error: "not_found",
        error_code: 404,
        message: "Unconfigured fixture API blocked locally",
      },
    });
  });
  await page.route("**/api/v1/users/me", (route) =>
    route.fulfill({
      json: {
        id: "test-user",
        email: "fixture@example.test",
        display_name: "Fixture owner",
        is_admin: false,
        is_active: true,
        email_verified: true,
        onboarding_completed: true,
        created_at: "2026-10-01T00:00:00Z",
        capabilities: {
          billing_available: true,
          ...(featureMode === "absent"
            ? {}
            : {
                enabled_features: [
                  "experimental:ai-assistant",
                  ...(featureMode === "on"
                    ? ["experimental:remote-machines"]
                    : []),
                ],
              }),
        },
      },
    }),
  );
  await page.route("**/api/v1/remote-machine**", async (route) => {
    const path = new URL(route.request().url()).pathname.replace("/api/v1", "");
    let body: unknown;
    if (path === "/remote-machine-entitlement")
      body = {
        eligible: true,
        free_slot_available: !created,
        machine_id: created ? machine.node_id : null,
        reason: null,
        policy_version: policy.policy_version,
        policy,
        platform: remoteMachinePlatformFixture,
      };
    else if (path === "/remote-machine-quotes")
      body = {
        policy_version: policy.policy_version,
        profile: machine.profile,
        free_slot_requested: true,
        paid_admission: "free",
        pricing_status: "free",
        valid_until: new Date(Date.now() + 900000).toISOString(),
        policy,
        platform: remoteMachinePlatformFixture,
      };
    else if (
      path === "/remote-machines" &&
      route.request().method() === "POST"
    ) {
      writes.push({
        body: route.request().postDataJSON(),
        key: route.request().headers()["idempotency-key"],
      });
      created = true;
      const configuration = route.request().postDataJSON();
      currentOperation = { ...remoteMachineOperationFixture, action: "create" };
      projection = {
        ...machine,
        name: configuration.name,
        capabilities: configuration.capabilities,
        separation_choice: configuration.separation_choice,
        separation_policy_version: configuration.separation_policy_version,
        specialist_id: configuration.specialist_id,
        resource_state: "provisioning",
        active_operation_id: currentOperation.id,
      };
      body = {
        machine: projection,
        operation: currentOperation,
        replayed: false,
      };
    } else if (path === "/remote-machines")
      body = { machines: created ? [projection] : [], next_cursor: null };
    else if (path === `/remote-machines/${machine.node_id}`)
      body = { machine: projection };
    else if (path.endsWith("/events")) body = { events: [], next_cursor: null };
    else if (path.endsWith("/usage"))
      body = {
        available: false,
        stale: true,
        received_at: null,
        workspace: null,
        temporary: null,
        disk_full: false,
        write_limit_bytes: 1048576,
        recovery: "Remove unneeded files through an authorized client.",
      };
    else if (
      path === `/remote-machine-operations/${remoteMachineOperationFixture.id}`
    )
      body = { operation: currentOperation };
    else
      return route.fulfill({
        status: 404,
        json: {
          error: "not_found",
          error_code: 404,
          message: "Fixture endpoint intentionally unavailable",
        },
      });
    await route.fulfill({ json: body });
  });
  await page.route("**/api/v1/billing/wallet", (route) =>
    route.fulfill({
      json: billingWallet({
        owner_id: "test-user",
        available: "12",
        available_credits: 12,
      }),
    }),
  );
  await page.route("**/api/v1/public/config", (route) =>
    route.fulfill({ json: { mcp_url: "https://fixture.example.test/mcp" } }),
  );
  await page.route("**/api/v1/assistant/nyxagent/agents**", (route) =>
    route.fulfill({ json: { agents: [] } }),
  );
  return writes;
}

for (const width of [1440, 390]) {
  for (const full of [false, true]) {
    test(`hosted ${full ? "full" : "Browser-only"} owner desktop navigation at ${width}px`, async ({
      page,
    }, testInfo) => {
      await page.setViewportSize({ width, height: 1000 });
      await machineApi(page, false, {
        capabilities: full
          ? ["shell", "files", "browser", "computer"]
          : ["browser"],
        resource_state: "ready",
        authorization_state: "active",
      });
      await page.route("**/api/v1/nodes", (route) =>
        route.fulfill({
          json: {
            nodes: [
              {
                id: machine.node_id,
                name: machine.name,
                owner: {
                  kind: "user",
                  id: "test-user",
                  display_name: "Fixture owner",
                },
                status: "online",
                is_connected: true,
                metadata: { provisioning_source: "remote_machine" },
                machine: {
                  version: 1,
                  roots: ["/workspace"],
                  os: "linux",
                  arch: "x86_64",
                  computer_mode: "standard",
                  cua_version: "1.2",
                  computer_tools: [],
                  browser_isolated: true,
                  commands_isolated: true,
                  saved_login_ready: false,
                  shell: full,
                  files: full,
                  browser: true,
                  computer: full,
                  computer_ready: true,
                  isolated: true,
                },
              },
            ],
          },
        }),
      );
      await page.route(
        "**/api/v1/assistant/nyxagent/machines/*/contexts",
        (route) => route.fulfill({ json: [] }),
      );
      const desktopRequests: string[] = [];
      await page.routeWebSocket(
        "**/api/v1/assistant/nyxagent/machines/*/desktop",
        (socket) => {
          desktopRequests.push(socket.url());
          socket.send(
            JSON.stringify({
              type: "error",
              error_code: 12417,
              message: "Computer permissions or display unavailable",
            }),
          );
        },
      );
      await page.goto("/assistant/machines");
      await page
        .getByRole("button", {
          name: `Settings for ${machine.name}`,
          exact: true,
        })
        .click();
      const detail = page.getByRole("dialog");
      const desktop = detail.getByRole("link", {
        name: "Open live desktop",
      });
      await expect(desktop).toHaveAttribute(
        "href",
        `/assistant/machines/${machine.node_id}/desktop`,
      );
      await noHorizontalOverflow(page);
      await page.screenshot({
        path: testInfo.outputPath("hosted-desktop-entry.png"),
        fullPage: true,
        animations: "disabled",
      });
      await desktop.click();
      await expect(page).toHaveURL(
        new RegExp(`/assistant/machines/${machine.node_id}/desktop$`),
      );
      const viewer = page.getByRole("region", { name: "Live machine desktop" });
      await expect(viewer).toBeVisible();
      await expect(viewer).toHaveAttribute("data-private", "true");
      await expect(
        viewer.getByText("Computer permissions or display unavailable", {
          exact: true,
        }),
      ).toBeVisible();
      // Development StrictMode replays mount effects. Every attempt must use
      // the existing owner desktop endpoint, without opening another surface.
      expect([...new Set(desktopRequests)]).toEqual([
        `ws://localhost:4627/api/v1/assistant/nyxagent/machines/${machine.node_id}/desktop`,
      ]);
      await noHorizontalOverflow(page);
    });
  }
}

async function openManagement(page: Page, name: string) {
  const summary = page.getByText("More machine details", { exact: true });
  if ((await summary.locator("..").getAttribute("open")) === null)
    await summary.click();
  await page.getByRole("button", { name, exact: true }).click();
}

async function noHorizontalOverflow(page: Page) {
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= window.innerWidth,
    ),
  ).toBe(true);
  const dialog = page.getByRole("dialog").last();
  if (await dialog.count())
    expect(
      await dialog.evaluate(
        (element) => element.scrollWidth <= element.clientWidth + 1,
      ),
    ).toBe(true);
}

for (const width of [1440, 390]) {
  test(`owner list and unavailable details remain accurate at ${width}px`, async ({
    page,
  }, testInfo) => {
    await page.setViewportSize({ width, height: 1000 });
    await machineApi(page);
    const errors: string[] = [];
    page.on("pageerror", (error) => errors.push(error.message));
    await page.goto("/assistant/machines");
    await expect(
      page.getByRole("region", { name: "Remote machines" }),
    ).toBeVisible();
    await noHorizontalOverflow(page);
    await page.screenshot({
      path: testInfo.outputPath("machines-list.png"),
      fullPage: true,
      animations: "disabled",
    });
    if (width < 768)
      await page
        .getByRole("button", {
          name: `Settings for ${machine.name}`,
          exact: true,
        })
        .click();
    else
      await page
        .getByRole("button", {
          name: `Settings for ${machine.name}`,
          exact: true,
        })
        .click();
    const detail = page.getByRole("dialog");
    await expect(detail.getByText(/Awaiting machine report/)).toBeVisible();
    await detail.getByText("Technical details", { exact: true }).click();
    await expect(
      detail.getByText(/No usage has been reported by the current runtime/),
    ).toBeVisible();
    await noHorizontalOverflow(page);
    await page.screenshot({
      path: testInfo.outputPath("machine-overview.png"),
      fullPage: true,
      animations: "disabled",
    });
    for (const name of ["Jobs", "Confirmations", "Billing"])
      await expect(
        detail.getByRole("button", { name, exact: true }),
      ).toHaveCount(0);
    expect(errors).toEqual([]);
  });

  test(`paid machine billing remains reachable at ${width}px`, async ({
    page,
  }) => {
    await page.setViewportSize({ width, height: 1000 });
    await machineApi(page, false, { free_slot: false, billing_state: "ready" });
    await page.goto(`/assistant/machines?machine=${machine.node_id}`);
    await openManagement(page, "Billing");
    const billing = page.getByRole("dialog", { name: "Billing", exact: true });
    await expect(
      billing.getByText(/Runtime billing details are unavailable/),
    ).toBeVisible();
    await expect(
      billing.getByText(/Historical contract lookup is unavailable/),
    ).toBeVisible();
    await noHorizontalOverflow(page);
  });

  test(`free creation requires policy review and a safe durable request at ${width}px`, async ({
    page,
  }, testInfo) => {
    await page.setViewportSize({ width, height: 1000 });
    const writes = await machineApi(page, true);
    await page.goto("/assistant/machines");
    await page
      .getByRole("button", { name: "Create remote machine", exact: true })
      .first()
      .click();
    await expect(page).toHaveURL(/\/assistant\/machines\/new\?where=hosted/);
    await expect(page.getByRole("tab")).toHaveCount(4);
    await expect(
      page.getByRole("tab", { name: "NyxID hosted" }),
    ).toHaveAttribute("aria-selected", "true");
    expect(
      await page.getByRole("tablist").evaluate((list) => {
        const bounds = list.getBoundingClientRect();
        return [...list.querySelectorAll("[role=tab]")].every((tab) => {
          const rect = tab.getBoundingClientRect();
          return rect.left >= bounds.left && rect.right <= bounds.right;
        });
      }),
    ).toBe(true);
    const create = page.getByRole("form", {
      name: "NyxID hosted machine configuration",
    });
    await expect(
      page.getByRole("button", { name: "Create my setup command" }),
    ).toHaveCount(0);
    const limits = create
      .locator("details")
      .filter({ hasText: "Limits and data policies" });
    const technical = create
      .locator("details")
      .filter({ hasText: "Technical details" });
    await expect(limits).not.toHaveAttribute("open");
    await expect(technical).not.toHaveAttribute("open");
    await expect(create.getByLabel("Machine specifications")).toHaveText(
      "2 vCPUCompute2 GiBMemory20 GiBWorkspaceLinuxSystem",
    );
    await expect(
      create.getByText(
        /Temporary storage is managed by the deployment environment/,
      ),
    ).toBeHidden();
    await expect(create.getByText(/2 GiB temporary storage/)).toHaveCount(0);
    await create.getByLabel("Machine name").fill("remote-research");
    await create.getByRole("checkbox", { name: /^Browser/ }).check();
    await expect(create.getByText("Available", { exact: true })).toBeVisible();
    await expect(
      create.getByText(/Paid creation requires a live synchronized quote/),
    ).toHaveCount(0);
    await expect(
      create.getByRole("button", { name: "Review paid creation" }),
    ).toHaveCount(0);
    await expect(
      create.getByRole("heading", { name: "Paid account-balance runtime" }),
    ).toHaveCount(0);
    await expect(create.getByText(/Account wallet available/)).toHaveCount(0);
    const submit = create.getByRole("button", {
      name: "Create free remote machine",
    });
    await expect(submit).toBeDisabled();
    expect(writes).toEqual([]);
    await noHorizontalOverflow(page);
    await page.screenshot({
      path: testInfo.outputPath("machine-create.png"),
      fullPage: true,
      animations: "disabled",
    });
    await limits.locator("summary").click();
    await technical.locator("summary").click();
    await expect(
      create.getByText(
        /Temporary storage is managed by the deployment environment/,
      ),
    ).toBeVisible();
    await expect(
      create.getByText(/Kubernetes native lifecycle observations/),
    ).toBeVisible();
    await noHorizontalOverflow(page);
    await page.screenshot({
      path: testInfo.outputPath("machine-create-expanded.png"),
      fullPage: true,
      animations: "disabled",
    });
    await limits.locator("summary").click();
    await technical.locator("summary").click();
    await create
      .getByRole("checkbox", { name: /I accept the shared environment/ })
      .check();
    await expect(submit).toBeDisabled();
    await create
      .getByRole("checkbox", { name: /I accept the selected Agent separation/ })
      .check();
    await expect(
      create.getByText(
        "Review and accept the selected Agent separation policy.",
        { exact: true },
      ),
    ).toHaveCount(0);
    await expect(
      create.getByText(
        "Review and accept the displayed profile, billing and data policies.",
        { exact: true },
      ),
    ).toHaveCount(0);
    await page.screenshot({
      path: testInfo.outputPath("machine-create-reviewed.png"),
      fullPage: true,
      animations: "disabled",
    });
    await submit.click();
    await expect.poll(() => writes.length).toBe(1);
    expect(writes[0]?.body).toEqual({
      name: "remote-research",
      capabilities: ["browser", "files", "shell"],
      specialist_id: null,
      separation_choice: "automatic",
      separation_policy_version: policy.separation_policy_version,
      acknowledge_separation: true,
      free_slot: true,
      profile_version: "remote-machine",
      policy_version: policy.policy_version,
      data_policy_version: policy.data_policy_version,
      acknowledge_shared_environment: true,
    });
    expect(writes[0]?.key).toMatch(/^[0-9a-f-]{36}$/);
    await expect
      .poll(() =>
        page.evaluate(
          () =>
            Object.keys(sessionStorage).filter((key) =>
              key.includes("remote-machine"),
            ).length,
        ),
      )
      .toBe(0);
    await expect(page).toHaveURL(
      new RegExp(`/assistant/machines\\?machine=${machine.node_id}$`),
    );
    await expect(
      page.getByRole("dialog", {
        name: "Current operation details",
        exact: true,
      }),
    ).toHaveCount(0);
    await expect(
      page.getByRole("heading", { name: "remote-research settings" }),
    ).toBeVisible();
    await expect(
      page.getByRole("heading", { name: "remote-research", exact: true }),
    ).toBeVisible();
    await page.getByText("Technical details", { exact: true }).click();
    await expect(
      page.getByText("No fixed capacity declared", { exact: true }).first(),
    ).toBeVisible();
    await noHorizontalOverflow(page);
    await page.goto(
      `/assistant/machines?machine=${machine.node_id}&operation=${remoteMachineOperationFixture.id}`,
    );
    const operation = page.getByRole("dialog", {
      name: "Current operation details",
      exact: true,
    });
    await expect(operation).toBeVisible();
    await expect(
      operation.getByText(
        /This read-only view does not change the current lifecycle operation/,
      ),
    ).toBeVisible();
    await expect(
      operation.getByRole("button", { name: "Retry deletion", exact: true }),
    ).toHaveCount(0);
    await noHorizontalOverflow(page);
  });
}

// These fixtures follow machine_external_service.rs. All requests remain
// intercepted; no live Node execution is exercised.
for (const width of [1440, 390]) {
  test(`confirmation deep link retains bound owner review at ${width}px`, async ({
    page,
  }, testInfo) => {
    await page.setViewportSize({ width, height: 1000 });
    await machineApi(page);
    const principal = {
      kind: "api_key",
      user_id: "test-user",
      api_key_id: "parent-key",
      api_key_credential_id: "child-key",
    };
    const pending = {
      id: "confirmation-live",
      node_id: machine.node_id,
      principal,
      grant_id: "grant-live",
      grant_revision: 7,
      assignment_revision: 4,
      runtime_id: "runtime-live",
      runtime_generation: 1,
      operation: "exec",
      parameters_digest: "a".repeat(64),
      safe_parameters: [
        { name: "Operation", value: "Exec" },
        {
          name: "Shell command",
          value: `8 bytes; content is not retained or shown; SHA-256 ${"b".repeat(64)}`,
        },
        { name: "Maximum command duration", value: "120 seconds" },
        {
          name: "Execution boundary",
          value:
            "Shared shell account; command text may perform additional actions. This summary does not verify the command's effects.",
        },
      ],
      review_parameters_available: true,
      review_content_available: false,
      state: "pending",
      created_at: "2026-10-13T20:00:00Z",
      expires_at: "2099-10-13T21:00:00Z",
      decided_at: null,
      owner_only: true,
    };
    let approved = false;
    const decisions: unknown[] = [];
    const listQueries: string[] = [];
    await page.route("**/api/v1/machine-confirmations?**", (route) => {
      listQueries.push(new URL(route.request().url()).search);
      return route.fulfill({
        json: {
          confirmations: [
            {
              ...pending,
              state: approved ? "approved" : "pending",
              decided_at: approved ? "2026-10-13T20:01:00Z" : null,
            },
          ],
        },
      });
    });
    await page.route(
      `**/api/v1/remote-machines/${machine.node_id}/confirmations/${pending.id}/decision`,
      (route) => {
        expect(route.request().method()).toBe("POST");
        decisions.push(route.request().postDataJSON());
        approved = true;
        return route.fulfill({
          json: {
            confirmation: {
              ...pending,
              subject: principal,
              status: "approved",
              approved_at: "2026-10-13T20:01:00Z",
              approved_until: pending.expires_at,
            },
          },
        });
      },
    );
    await page.goto(
      `/assistant/machines?machine=${machine.node_id}&confirmation=${pending.id}`,
    );
    await page
      .getByRole("button", { name: "Review decision", exact: true })
      .click();
    const review = page.getByRole("dialog", {
      name: "Review external request",
    });
    await expect(
      review.getByText(/Parent Key parent-key · Child child-key/),
    ).toBeVisible();
    await expect(
      review.getByText(/8 bytes; content is not retained or shown/),
    ).toBeVisible();
    const approve = review.getByRole("button", {
      name: "Approve one retry",
      exact: true,
    });
    await expect(approve).toBeDisabled();
    expect(decisions).toHaveLength(0);
    await review
      .getByRole("checkbox", {
        name: "I reviewed this identity and safe request summary.",
      })
      .check();
    await expect(approve).toBeEnabled();
    await page.screenshot({
      path: testInfo.outputPath("machine-confirmation-bound-review.png"),
      fullPage: true,
      animations: "disabled",
    });
    await approve.click();
    await expect(review).toHaveCount(0);
    await expect(page.getByText("approved", { exact: true })).toBeVisible();
    expect(
      listQueries.every(
        (query) => query === `?node_id=${machine.node_id}&limit=100`,
      ),
    ).toBe(true);
    expect(decisions).toEqual([
      {
        approve: true,
        expected_grant_revision: 7,
        expected_runtime_generation: 1,
        parameters_digest: pending.parameters_digest,
      },
    ]);
    await noHorizontalOverflow(page);
  });
}

for (const featureMode of ["absent", "off"] as const) {
  test(`personal flag ${featureMode} blocks direct hosted URL and creation CTA but keeps management`, async ({
    page,
  }) => {
    const admissions: string[] = [];
    page.on("request", (request) => {
      const path = new URL(request.url()).pathname;
      if (request.method() !== "GET" && path.includes("/remote-machine"))
        admissions.push(path);
    });
    await machineApi(page, false, {}, featureMode);
    await page.goto("/assistant/machines/new?where=hosted");
    await expect(
      page.getByRole("heading", { name: "Add a machine" }),
    ).toBeVisible();
    await expect(page.getByRole("tab", { name: "NyxID hosted" })).toHaveCount(
      0,
    );
    for (const name of ["This computer", "Remote VM", "Docker"])
      await expect(page.getByRole("tab", { name })).toBeVisible();
    await expect(
      page.getByRole("form", { name: "NyxID hosted machine configuration" }),
    ).toHaveCount(0);
    await page.goto("/assistant/machines");
    await expect(
      page.getByRole("button", { name: "Create remote machine", exact: true }),
    ).toHaveCount(0);
    await expect(
      page.getByRole("button", { name: "Add a machine", exact: true }),
    ).toBeVisible();
    await page.goto(`/assistant/machines?machine=${machine.node_id}`);
    await expect(
      page.getByRole("button", { name: "Resume machine", exact: true }),
    ).toBeDisabled();
    await expect(
      page.getByRole("button", { name: "Delete machine", exact: true }),
    ).toBeEnabled();
    expect(admissions).toEqual([]);
  });
}

const evidenceRoot =
  process.env.P4_E2E_EVIDENCE_DIR ??
  "../docs/evidence/machine-rollout-20261026";
const presentation: NonNullable<RemoteMachine["presentation"]> = {
  agent_version: "0.73.1",
  os: "linux",
  arch: "x86_64",
  roots: ["/workspace"],
  capabilities: ["shell", "files", "browser", "computer"],
  commands_isolated: true,
  browser_isolated: false,
  separation_support: true,
  owner_confirmation: "changes",
  allow_single_user_saved_logins: true,
  computer_mode: "standard",
  cua_version: "0.30.4",
  connected: true,
  desktop_ready: true,
  historical: false,
  configuration_current: true,
  last_reported_at: "2026-10-26T00:00:00Z",
};
const fixtureLimits = {
  skip_destructive_confirmation: false,
  max_live_subagents: 3,
  max_concurrent_subagent_turns: 3,
  max_live_subagents_limit: 6,
  max_concurrent_subagent_turns_limit: 6,
};
const specialist = (id: string, name: string) => ({
  id,
  name,
  kind: "specialist",
  status: "idle",
  owner_id: "test-user",
  owner_kind: "person",
  services: ["github"],
  account_read: true,
  machines: [],
  logins: [],
  guest_access: {},
  created_at: "2026-10-26T00:00:00Z",
  last_active_at: "2026-10-26T00:00:00Z",
});
async function visualFixture(page: Page) {
  await page.addInitScript(() => {
    Object.defineProperty(globalThis, "__nyxidAssistantHttpMock", {
      get: () => undefined,
      set: () => {},
      configurable: true,
    });
  });
  await machineApi(page, false, {
    resource_state: "ready",
    authorization_state: "active",
    presentation,
    capabilities: presentation.capabilities,
    machine_configuration: {
      ...machine.machine_configuration,
      state: "applied",
      reason: null,
    },
  });
  await page.route("**/api/v1/nodes", (route) =>
    route.fulfill({
      json: {
        nodes: [
          {
            id: "self-hosted",
            name: "Linux workstation",
            is_connected: true,
            status: "online",
            owner: {
              id: "test-user",
              kind: "user",
              display_name: "Personal account",
            },
            machine_confirm: "changes",
            allow_single_user_saved_logins: false,
            metadata: { agent_version: "0.73.1" },
            machine: {
              version: 1,
              shell: true,
              files: true,
              browser: true,
              computer: true,
              os: "linux",
              arch: "x86_64",
              roots: ["/workspace"],
              computer_mode: "standard",
              cua_version: "0.30.4",
              computer_tools: [],
              computer_ready: true,
              browser_isolated: false,
              commands_isolated: true,
              saved_login_ready: false,
            },
          },
        ],
      },
    }),
  );
  await page.route("**/api/v1/machines/updates", (route) =>
    route.fulfill({ json: [] }),
  );
  await page.route("**/api/v1/assistant/nyxagent/agents**", (route) =>
    route.fulfill({
      json: {
        agents: [
          specialist("coder", "Code assistant"),
          specialist("research", "Research assistant"),
        ],
        limits: fixtureLimits,
      },
    }),
  );
  await page.route("**/api/v1/machines/*/activity**", (route) =>
    route.fulfill({
      json: {
        machine_name: "Research workspace",
        agents: [],
        entries: [
          {
            id: "activity-1",
            agent_id: null,
            actor_id: "test-user",
            action: "machine.exec",
            outcome: "completed",
            operation_id: "tool-op",
            activity_id: "activity-1",
            job_id: "job-1",
            exit_code: 0,
            duration_ms: 120,
            created_at: "2026-10-26T00:00:00Z",
          },
        ],
        next_cursor: null,
      },
    }),
  );
  const login = {
    id: "login-1",
    owner_id: "test-user",
    label: "GitHub work",
    allowed_origins: ["https://github.com"],
    username_hint: "j***",
    has_password: true,
    has_totp: false,
    confirm_each_sign_in: true,
    created_at: "2026-10-26T00:00:00Z",
    updated_at: "2026-10-26T00:00:00Z",
    last_used_at: null,
  };
  const loginWrites: string[] = [];
  await page.route("**/api/v1/saved-logins**", (route) => {
    if (route.request().method() !== "GET") {
      loginWrites.push(route.request().method());
      return route.fulfill({
        status: 405,
        json: { error: "method_not_allowed" },
      });
    }
    return route.fulfill({ json: [login] });
  });
  return loginWrites;
}
async function evidence(page: Page, name: string) {
  await noHorizontalOverflow(page);
  await page.screenshot({
    path: `${evidenceRoot}/${name}.png`,
    fullPage: true,
    animations: "disabled",
  });
}
for (const width of [1440, 390])
  for (const theme of ["light", "dark"] as const) {
    test(`production and preview acceptance ${width}px ${theme}`, async ({
      page,
    }) => {
      test.setTimeout(60000);
      page.setDefaultTimeout(10000);
      await page.setViewportSize({ width, height: 1000 });
      await page.emulateMedia({ colorScheme: theme, reducedMotion: "reduce" });
      await page.addInitScript(
        (theme) =>
          localStorage.setItem(
            "nyxid.theme",
            JSON.stringify({ state: { mode: theme }, version: 1 }),
          ),
        theme,
      );
      const loginWrites = await visualFixture(page);
      const suffix = `${width}-${theme}`;
      await page.goto("/assistant/machines");
      await expect(
        page.getByRole("article", { name: "Linux workstation" }),
      ).toBeVisible();
      await evidence(page, `production-list-${suffix}`);
      await page
        .getByRole("button", {
          name: `Settings for ${machine.name}`,
          exact: true,
        })
        .click();
      await expect(
        page.getByRole("region", { name: "Version", exact: true }),
      ).toContainText("0.73.1");
      await expect(
        page.getByRole("combobox", { name: "Owner confirmation" }),
      ).toBeDisabled();
      await expect(
        page.getByRole("link", { name: "Open live desktop" }),
      ).toBeVisible();
      await expect(
        page.getByRole("button", { name: "Save machine settings" }),
      ).toBeInViewport();
      for (const name of [
        "Jobs",
        "Confirmations",
        "Billing",
        "Review agent access",
        "Check status",
      ])
        await expect(
          page.getByRole("button", { name, exact: true }),
        ).toHaveCount(0);
      await expect(
        page.getByText("Specialist assigned", { exact: true }),
      ).toHaveCount(0);
      await expect(
        page.getByRole("link", { name: "Open specialist settings" }),
      ).toHaveCount(0);
      await expect(
        page.getByText(/^Hosted configuration applied/),
      ).not.toBeVisible();
      await evidence(page, `production-remote-settings-${suffix}`);
      await page
        .getByRole("checkbox", { name: "Code assistant", exact: true })
        .check();
      await evidence(page, `production-specialist-draft-${suffix}`);
      await page
        .getByRole("button", { name: "Manage saved logins", exact: true })
        .click();
      await expect(
        page.getByText("GitHub work", { exact: true }),
      ).toBeVisible();
      await expect(
        page.getByRole("button", { name: "Edit GitHub work", exact: true }),
      ).toHaveCount(0);
      await expect(
        page.getByRole("textbox", { name: "Login label" }),
      ).toHaveCount(0);
      expect(loginWrites).toEqual([]);
      await evidence(page, `production-account-saved-logins-${suffix}`);
      await page
        .getByRole("button", { name: "Back to machine settings", exact: true })
        .click();
      await expect(
        page.getByRole("checkbox", { name: "Code assistant", exact: true }),
      ).toBeChecked();
      await page
        .getByRole("button", { name: "Discard changes", exact: true })
        .click();
      await expect(
        page.getByRole("checkbox", { name: "Code assistant", exact: true }),
      ).not.toBeChecked();
      await page.getByRole("button", { name: "Close", exact: true }).click();
      await page
        .getByRole("button", {
          name: "Settings for Linux workstation",
          exact: true,
        })
        .click();
      await expect(
        page.getByRole("region", { name: "Version and updates", exact: true }),
      ).toBeVisible();
      await evidence(page, `production-self-settings-${suffix}`);
      await page.getByRole("button", { name: "Close", exact: true }).click();
      await page
        .getByRole("button", {
          name: `Activity for ${machine.name}`,
          exact: true,
        })
        .click();
      await expect(
        page.getByText("machine · exec", { exact: true }),
      ).toBeVisible();
      await evidence(page, `production-tool-activity-${suffix}`);
      await page
        .getByRole("tab", { name: "Machine events", exact: true })
        .click();
      await evidence(page, `production-machine-events-${suffix}`);
      await page.getByRole("button", { name: "Close", exact: true }).click();
      await page.goto("/assistant/machines/new?where=hosted");
      await expect(
        page.getByRole("link", { name: "View machine", exact: true }),
      ).toBeVisible();
      await expect(
        page.getByRole("form", { name: "NyxID hosted machine configuration" }),
      ).toHaveCount(0);
      await evidence(page, `production-occupied-${suffix}`);
      await page.route("**/api/v1/remote-machine-entitlement**", (route) =>
        route.fulfill({
          json: {
            eligible: true,
            free_slot_available: true,
            machine_id: null,
            reason: null,
            policy_version: policy.policy_version,
            policy,
            platform: remoteMachinePlatformFixture,
          },
        }),
      );
      await page.reload();
      await expect(
        page.getByRole("textbox", { name: "Machine name" }),
      ).toHaveValue("my-remote-machine");
      await expect(
        page.getByRole("checkbox", {
          name: /I accept the selected Agent separation/,
        }),
      ).toBeEnabled();
      await expect(
        page.getByText(
          "Review and accept the selected Agent separation policy.",
          { exact: true },
        ),
      ).toHaveCount(0);
      await evidence(page, `production-create-${suffix}`);
      await page
        .getByRole("group", { name: "Enable on this machine" })
        .scrollIntoViewIfNeeded();
      await evidence(page, `production-create-fields-${suffix}`);
      await page.getByRole("tab", { name: "Remote VM", exact: true }).click();
      await evidence(page, `production-self-create-${suffix}`);
      await page.goto("/machine-preview.html?view=detail&scenario=ready");
      await expect(
        page.getByRole("region", { name: "Version", exact: true }),
      ).toBeVisible();
      await evidence(page, `preview-remote-settings-${suffix}`);
      await page.goto("/machine-preview.html?view=machines&scenario=ready");
      await expect(
        page.getByRole("heading", { name: "Machines", exact: true }),
      ).toBeVisible();
      await evidence(page, `preview-list-${suffix}`);
      await page.goto(
        "/machine-preview.html?view=create&scenario=create&where=hosted",
      );
      await expect(
        page.getByRole("heading", {
          name: "Create remote machine",
          exact: true,
        }),
      ).toBeVisible();
      await evidence(page, `preview-create-${suffix}`);
    });
  }

test("unknown lifecycle result keeps the reviewed request through close and refresh, and stale revision requires new review", async ({
  page,
}) => {
  await visualFixture(page);
  let current: RemoteMachine = {
    ...machine,
    resource_state: "ready",
    authorization_state: "active",
    presentation,
    capabilities: presentation.capabilities,
  };
  let reads = 0;
  const actions: { body: unknown; key: string | undefined }[] = [];
  await page.route(`**/api/v1/remote-machines/${machine.node_id}`, (route) => {
    reads++;
    return route.fulfill({ json: { machine: current } });
  });
  await page.route(
    `**/api/v1/remote-machines/${machine.node_id}/actions`,
    (route) => {
      actions.push({
        body: route.request().postDataJSON(),
        key: route.request().headers()["idempotency-key"],
      });
      if (actions.length === 1) return route.abort("failed");
      if (actions.length === 2) {
        current = { ...current, revision: current.revision + 1 };
        return route.fulfill({
          status: 409,
          json: {
            error: "conflict",
            error_code: 409,
            message: "Review the latest machine revision.",
          },
        });
      }
      current = {
        ...current,
        resource_state: "stopping",
        active_operation_id: remoteMachineOperationFixture.id,
      };
      return route.fulfill({
        json: {
          machine: current,
          operation: remoteMachineOperationFixture,
          replayed: false,
        },
      });
    },
  );
  await page.goto(`/assistant/machines?machine=${machine.node_id}`);
  await page
    .getByRole("button", { name: "Pause machine", exact: true })
    .click();
  let review = page.getByRole("dialog", {
    name: "Pause machine?",
    exact: true,
  });
  await expect(review.getByRole("checkbox")).toHaveCount(0);
  await review
    .getByRole("button", { name: "Pause machine", exact: true })
    .click();
  await expect(review.getByText(/The request result is unknown/)).toBeVisible();
  await expect(
    review.getByRole("button", { name: "Pause machine", exact: true }),
  ).toBeDisabled();
  expect(actions).toHaveLength(1);
  await review.getByRole("button", { name: "Cancel", exact: true }).click();
  await page.getByRole("button", { name: "Close", exact: true }).click();
  const unknownCard = page.getByRole("article", { name: machine.name! });
  await expect(
    unknownCard.getByText(/previous pause request has an unconfirmed result/),
  ).toBeVisible();
  await page.reload();
  await expect(
    unknownCard.getByText(/previous pause request has an unconfirmed result/),
  ).toBeVisible();
  expect(actions).toHaveLength(1);
  await unknownCard
    .getByRole("button", { name: `Settings for ${machine.name}`, exact: true })
    .click();
  await page
    .getByRole("button", { name: "Pause machine", exact: true })
    .click();
  review = page.getByRole("dialog", { name: "Pause machine?", exact: true });
  await expect(
    review.getByRole("button", { name: "Pause machine", exact: true }),
  ).toBeDisabled();
  expect(actions).toHaveLength(1);
  const readsBefore = reads;
  await review
    .getByRole("button", { name: "Check request status", exact: true })
    .click();
  await expect.poll(() => reads).toBeGreaterThan(readsBefore);
  await expect(
    review.getByRole("button", { name: "Pause machine", exact: true }),
  ).toBeEnabled();
  await review
    .getByRole("button", { name: "Pause machine", exact: true })
    .click();
  await expect(
    review.getByRole("button", {
      name: "Reload and review a new action",
      exact: true,
    }),
  ).toBeVisible();
  await expect(
    review.getByRole("button", { name: "Pause machine", exact: true }),
  ).toBeDisabled();
  expect(actions).toHaveLength(2);
  expect(actions[1]).toEqual(actions[0]);
  expect(actions[0]?.body).toEqual({ action: "pause", expected_revision: 1 });
  await review
    .getByRole("button", {
      name: "Reload and review a new action",
      exact: true,
    })
    .click();
  await expect(review).toHaveCount(0);
  await page
    .getByRole("button", { name: "Pause machine", exact: true })
    .click();
  review = page.getByRole("dialog", { name: "Pause machine?", exact: true });
  await review
    .getByRole("button", { name: "Pause machine", exact: true })
    .click();
  await expect.poll(() => actions.length).toBe(3);
  expect(actions[2]?.body).toEqual({ action: "pause", expected_revision: 2 });
  expect(actions[2]?.key).not.toEqual(actions[0]?.key);
  expect(actions[2]?.key).toMatch(/^[0-9a-f-]{36}$/);
  await expect(
    page.getByRole("dialog", {
      name: "Current operation details",
      exact: true,
    }),
  ).toHaveCount(0);
});

for (const action of ["pause", "resume", "delete"] as const) {
  test(`accepted lifecycle continuity for ${action} survives list, Activity and reload with stale reads`, async ({ page }) => {
    await visualFixture(page);
    let current: RemoteMachine = { ...machine, resource_state: action === "resume" ? "paused" : "ready", active_operation_id: "previous-operation" };
    let detailCurrent: RemoteMachine | undefined;
    const previous: RemoteMachineOperation = { ...remoteMachineOperationFixture, id: "previous-operation", action: "resume", state: "failed", error_message: "Previous operation failure", deadline_at: "2020-01-01T00:00:00Z", terminal_at: "2026-10-26T00:00:00Z" };
    let accepted: RemoteMachineOperation = { ...remoteMachineOperationFixture, id: `accepted-${action}`, machine_id: machine.node_id, action, state: "accepted", deadline_at: "2099-01-01T00:00:00Z", terminal_at: null, error_message: null };
    let submissions = 0;
    await page.route("**/api/v1/remote-machines", (route) => route.fulfill({ json: { machines: [current], next_cursor: null } }));
    await page.route(`**/api/v1/remote-machines/${machine.node_id}`, (route) => {
      if (route.request().method() === "GET") return route.fulfill({ json: { machine: detailCurrent ?? current } });
      submissions++;
      return route.fulfill({ json: { operation: accepted } });
    });
    await page.route(`**/api/v1/remote-machines/${machine.node_id}/actions`, (route) => {
      submissions++;
      return route.fulfill({ json: { operation: accepted } });
    });
    await page.route("**/api/v1/remote-machine-operations/*", (route) => route.fulfill({ json: { operation: route.request().url().endsWith(accepted.id) ? accepted : previous } }));
    const label = action === "pause" ? "Pause machine" : action === "resume" ? "Resume machine" : "Delete machine";
    const heading = action === "pause" ? "Pausing your machine" : action === "resume" ? "Resuming your machine" : "Deleting your machine";
    await page.goto(`/assistant/machines?machine=${machine.node_id}`);
    await page.getByRole("button", { name: label, exact: true }).click();
    const review = page.getByRole("dialog", { name: `${label}?`, exact: true });
    if (action === "delete") await review.getByRole("checkbox").check();
    await review.getByRole("button", { name: label, exact: true }).click();
    await expect(page.getByRole("region", { name: "Operation progress" }).getByText(heading, { exact: true })).toBeVisible();
    await page.getByRole("button", { name: "Close", exact: true }).click();
    const card = page.getByRole("article", { name: machine.name });
    await expect(card).toContainText(heading);
    await card.getByRole("button", { name: `Settings for ${machine.name}`, exact: true }).click();
    await expect(page.getByText(heading, { exact: true })).toBeVisible();
    await page.getByRole("button", { name: "View activity", exact: true }).click();
    await expect(page.getByRole("heading", { name: "Machine activity", exact: true })).toBeVisible();
    await page.getByRole("button", { name: "Close", exact: true }).click();
    await card.getByRole("button", { name: `Settings for ${machine.name}`, exact: true }).click();
    await page.reload();
    const progress = page.getByRole("region", { name: "Operation progress" });
    await expect(progress.getByText(heading, { exact: true })).toBeVisible();
    await expect(progress.getByText("Previous operation failure")).toHaveCount(0);
    await expect(progress.getByText("Taking longer than usual")).toHaveCount(0);
    expect(submissions).toBe(1);
    current = { ...current, revision: current.revision + 1, active_operation_id: accepted.id, resource_state: action === "pause" ? "paused" : action === "resume" ? "ready" : "deleted", cleanup_state: action === "delete" ? "complete" : "not_required" };
    accepted = { ...accepted, state: "succeeded", terminal_at: "2026-10-29T00:00:00Z" };
    await page.reload();
    await expect(page.getByRole("region", { name: "Operation progress" })).toHaveCount(0);
    expect(submissions).toBe(1);
    if (action === "pause") {
      const originalRequest = await page.evaluate(() => {
        const raw = Object.entries(sessionStorage).find(([key]) => key.endsWith(":accepted"))?.[1];
        const receipt = JSON.parse(raw!);
        return { operationId: receipt.operationId, action: receipt.action, body: receipt.body, idempotencyKey: receipt.idempotencyKey };
      });
      // The list sees later work from another client, while detail has its own
      // older query result. That newer association must survive every consumer.
      detailCurrent = { ...current };
      for (const laterAction of ["resume", "delete"] as const) {
        await page.getByRole("button", { name: "Close", exact: true }).click();
        accepted = { ...accepted, id: `other-client-${laterAction}`, action: laterAction, state: "applying", terminal_at: null };
        current = { ...current, revision: current.revision + 1, active_operation_id: accepted.id, resource_state: laterAction === "resume" ? "resuming" : "deleting" };
        await page.reload();
        const laterHeading = laterAction === "resume" ? "Resuming your machine" : "Deleting your machine";
        await expect(card).toContainText(laterHeading);
        await card.getByRole("button", { name: `Settings for ${machine.name}`, exact: true }).click();
        await expect(page.getByRole("region", { name: "Operation progress" }).getByText(laterHeading, { exact: true })).toBeVisible();
        await page.getByRole("button", { name: "View activity", exact: true }).click();
        await page.getByRole("button", { name: "Close", exact: true }).click();
        await card.getByRole("button", { name: `Settings for ${machine.name}`, exact: true }).click();
        await page.reload();
        const laterProgress = page.getByRole("region", { name: "Operation progress" });
        await expect(laterProgress.getByText(laterHeading, { exact: true })).toBeVisible();
        await expect(laterProgress.getByText("Previous operation failure")).toHaveCount(0);
        await expect(laterProgress.getByText("Taking longer than usual")).toHaveCount(0);
        const retained = await page.evaluate(() => {
          const receipt = JSON.parse(Object.entries(sessionStorage).find(([key]) => key.endsWith(":accepted"))![1]);
          return { operationId: receipt.operationId, action: receipt.action, body: receipt.body, idempotencyKey: receipt.idempotencyKey, currentId: receipt.currentObservation.operationId };
        });
        expect(retained).toEqual({ ...originalRequest, currentId: accepted.id });
        expect(submissions).toBe(1);
        if (laterAction === "resume") {
          accepted = { ...accepted, state: "failed", terminal_at: "2026-10-29T00:00:00Z", error_message: "Current operation failed" };
          detailCurrent = { ...current };
          await page.reload();
          await expect(page.getByRole("region", { name: "Operation progress" })).toContainText("Current operation failed");
          await expect(page.getByRole("button", { name: "Resuming…", exact: true })).toBeDisabled();
          await expect(page.getByRole("dialog", { name: "Resume machine?", exact: true })).toHaveCount(0);
          await page.getByRole("button", { name: "Close", exact: true }).click();
          await expect(card).toContainText("The operation needs attention");
          await expect(card.getByRole("button", { name: "Resuming…", exact: true })).toBeDisabled();
          await card.getByRole("button", { name: `Settings for ${machine.name}`, exact: true }).click();
          expect(submissions).toBe(1);
        }
        // The Operation can finish before its independently read resource.
        // Equal current ID/revision is association, not resource finality.
        accepted = { ...accepted, state: "succeeded", terminal_at: "2026-10-29T00:00:00Z", error_message: null };
        detailCurrent = { ...current };
        await page.reload();
        await expect(page.getByRole("region", { name: "Operation progress" }).getByText(laterHeading, { exact: true })).toBeVisible();
        if (laterAction === "resume")
          await expect(page.getByRole("button", { name: "Resuming…", exact: true })).toBeDisabled();
        await page.getByRole("button", { name: "Close", exact: true }).click();
        await expect(card).toContainText(laterHeading);
        await card.getByRole("button", { name: `Settings for ${machine.name}`, exact: true }).click();
        await expect(page.getByRole("region", { name: "Operation progress" }).getByText(laterHeading, { exact: true })).toBeVisible();
        current = { ...current, revision: current.revision + 1, resource_state: laterAction === "resume" ? "ready" : "deleted", cleanup_state: laterAction === "delete" ? "complete" : "not_required" };
        detailCurrent = { ...current };
        await page.reload();
        await expect(page.getByRole("region", { name: "Operation progress" })).toHaveCount(0);
        if (laterAction === "resume")
          await expect(page.getByRole("button", { name: "Pause machine", exact: true })).toBeEnabled();
        expect(submissions).toBe(1);
      }
    }
  });
}

test("lifecycle intent and original delete recovery survive drawer close and reload", async ({
  page,
}) => {
  test.setTimeout(90000);
  page.setDefaultTimeout(10000);
  await page.setViewportSize({ width: 390, height: 1000 });
  await page.emulateMedia({ colorScheme: "dark", reducedMotion: "reduce" });
  await page.addInitScript(() =>
    localStorage.setItem(
      "nyxid.theme",
      JSON.stringify({ state: { mode: "dark" }, version: 1 }),
    ),
  );
  await visualFixture(page);
  let current: RemoteMachine = {
    ...machine,
    resource_state: "ready",
    authorization_state: "active",
    capabilities: presentation.capabilities,
    presentation,
  };
  let operation: RemoteMachineOperation = {
    ...remoteMachineOperationFixture,
    id: "original-lifecycle",
    action: "pause" as const,
    state: "applying" as const,
  };
  await page.route("**/api/v1/remote-machines", (route) =>
    route.fulfill({
      json: {
        machines: current.resource_state === "deleted" ? [] : [current],
        next_cursor: null,
      },
    }),
  );
  let operationReads = 0;
  const actions: {
    method: string;
    body: Record<string, unknown>;
    key: string | undefined;
  }[] = [];
  const retries: { body: unknown; key: string | undefined }[] = [];
  await page.route(`**/api/v1/remote-machines/${machine.node_id}`, (route) => {
    if (route.request().method() === "GET")
      return route.fulfill({ json: { machine: current } });
    actions.push({
      method: route.request().method(),
      body: route.request().postDataJSON(),
      key: route.request().headers()["idempotency-key"],
    });
    operation = {
      ...remoteMachineOperationFixture,
      id: "original-deletion",
      action: "delete",
      state: "reconciling",
      cleanup_state: "pending",
      retry_count: 10,
      delete_retry_attempts: 4,
      delete_retry_limit: 10,
      delete_retry_stopped: false,
      next_attempt_at: "2099-01-01T00:00:00Z",
    } as typeof operation;
    current = {
      ...current,
      resource_state: "deleting",
      authorization_state: "revoked",
      cleanup_state: "pending",
      active_operation_id: operation.id,
      revision: current.revision + 1,
      presentation: {
        ...presentation,
        connected: false,
        desktop_ready: false,
        historical: true,
      },
    };
    return route.fulfill({
      json: { machine: current, operation, replayed: false },
    });
  });
  await page.route(
    `**/api/v1/remote-machines/${machine.node_id}/actions`,
    (route) => {
      const body = route.request().postDataJSON();
      actions.push({
        method: route.request().method(),
        body,
        key: route.request().headers()["idempotency-key"],
      });
      operation = {
        ...remoteMachineOperationFixture,
        id: `original-${body.action}`,
        action: body.action,
        state: "applying",
      };
      current = {
        ...current,
        resource_state: body.action === "pause" ? "stopping" : "resuming",
        authorization_state: "revoked",
        active_operation_id: operation.id,
        revision: current.revision + 1,
        presentation: {
          ...presentation,
          connected: false,
          desktop_ready: false,
          historical: true,
        },
      };
      return route.fulfill({
        json: { machine: current, operation, replayed: false },
      });
    },
  );
  await page.route("**/api/v1/remote-machine-operations/*", (route) => {
    operationReads++;
    return route.fulfill({ json: { operation } });
  });
  await page.route(
    "**/api/v1/remote-machine-operations/*/retry-delete",
    (route) => {
      retries.push({
        body: route.request().postDataJSON(),
        key: route.request().headers()["idempotency-key"],
      });
      if (retries.length === 1)
        return route.fulfill({
          status: 400,
          json: {
            error: "bad_request",
            error_code: 400,
            message: "Retry request rejected by fixture",
          },
        });
      if (retries.length === 2) return route.abort("failed");
      operation = {
        ...operation,
        delete_retry_stopped: false,
        delete_retry_attempts: 0,
        next_attempt_at: "2099-01-01T00:00:00Z",
      } as typeof operation;
      return route.fulfill({ json: { operation, replayed: true } });
    },
  );
  await page.goto(`/assistant/machines?machine=${machine.node_id}`);
  await page
    .getByRole("button", { name: "Pause machine", exact: true })
    .click();
  let review = page.getByRole("dialog", {
    name: "Pause machine?",
    exact: true,
  });
  await evidence(page, "production-pause-confirm-390-dark");
  await expect(review.getByRole("checkbox")).toHaveCount(0);
  await review
    .getByRole("button", { name: "Pause machine", exact: true })
    .click();
  await expect.poll(() => actions.length).toBe(1);
  await expect(
    page.getByText("Pausing your machine", { exact: true }),
  ).toBeVisible();
  const pauseProgress = page.getByRole("region", {
    name: "Operation progress",
  });
  await expect(pauseProgress).toHaveCount(1);
  await expect(pauseProgress.getByRole("listitem")).toHaveCount(3);
  await expect(
    pauseProgress.getByLabel("Confirmed", { exact: true }),
  ).toHaveCount(1);
  await expect(
    page.getByRole("button", { name: "Pausing…", exact: true }),
  ).toBeDisabled();
  await expect(
    page.getByRole("dialog", { name: "Current operation details" }),
  ).toHaveCount(0);
  await page.getByRole("button", { name: "Close", exact: true }).click();
  await expect(page.getByRole("article", { name: machine.name })).toContainText(
    "stopping",
  );
  current = { ...current, resource_state: "paused", active_operation_id: null };
  operation = { ...operation, state: "succeeded", terminal_at: "2026-10-29T00:00:00Z" };
  await page.goto(`/assistant/machines?machine=${machine.node_id}`);
  await expect(
    page.getByRole("button", { name: "Resume machine", exact: true }),
  ).toBeEnabled();
  await evidence(page, "production-paused-390-dark");
  await page
    .getByRole("button", { name: "Resume machine", exact: true })
    .click();
  review = page.getByRole("dialog", { name: "Resume machine?", exact: true });
  await evidence(page, "production-resume-confirm-390-dark");
  await expect(review.getByRole("checkbox")).toHaveCount(0);
  await review
    .getByRole("button", { name: "Resume machine", exact: true })
    .click();
  await expect.poll(() => actions.length).toBe(2);
  current = { ...current, resource_state: "reconciling" };
  await page.reload();
  await evidence(page, "production-resume-delayed-390-dark");
  operation = {
    ...operation,
    state: "failed",
    terminal_at: "2026-10-26T00:00:00Z",
    error_code: "runtime_start_failed",
    error_message: "Runtime could not be started; workspace retained.",
  } as typeof operation;
  current = { ...current, resource_state: "paused" };
  await page.reload();
  await expect(
    page.getByText("Runtime could not be started; workspace retained.", {
      exact: true,
    }),
  ).toBeVisible();
  await evidence(page, "production-resume-failed-390-dark");
  await page
    .getByRole("button", { name: "Delete machine", exact: true })
    .click();
  review = page.getByRole("dialog", {
    name: "Delete machine?",
    exact: true,
  });
  await evidence(page, "production-delete-confirm-390-dark");
  await review.getByRole("checkbox", { name: /I understand/ }).check();
  await review
    .getByRole("button", { name: "Delete machine", exact: true })
    .click();
  await expect.poll(() => actions.length).toBe(3);
  await expect(
    page.getByRole("heading", {
      name: "Taking longer than usual",
      exact: true,
    }),
  ).toBeVisible();
  await expect(page.getByText(/Deletion is still pending/)).toBeVisible();
  await expect(
    page.getByRole("region", { name: "Operation progress" }),
  ).toHaveCount(1);
  await evidence(page, "production-delete-delayed-390-dark");
  operation = {
    ...operation,
    delete_retry_stopped: true,
    delete_retry_attempts: 10,
    next_attempt_at: null,
  } as typeof operation;
  await page.reload();
  await expect(
    page.getByRole("button", { name: "Retry deletion", exact: true }),
  ).toBeVisible();
  await evidence(page, "production-delete-stopped-390-dark");
  await page
    .getByRole("button", { name: "Retry deletion", exact: true })
    .click();
  await expect(
    page.getByText("Retry request rejected by fixture", { exact: true }),
  ).toBeVisible();
  await evidence(page, "production-delete-retry-failed-390-dark");
  await page
    .getByRole("button", { name: "Retry deletion", exact: true })
    .click();
  await expect(page.getByText(/Deletion retry result unknown/)).toBeVisible();
  await evidence(page, "production-delete-retry-unknown-390-dark");
  await page.getByRole("button", { name: "Close", exact: true }).click();
  await page.goto(`/assistant/machines?machine=${machine.node_id}`);
  await page.reload();
  const readsBefore = operationReads;
  await page
    .getByRole("button", { name: "Check retry status", exact: true })
    .click();
  await expect.poll(() => operationReads).toBeGreaterThan(readsBefore);
  expect(retries).toHaveLength(2);
  await page
    .getByRole("button", { name: "Retry deletion", exact: true })
    .click();
  await expect.poll(() => retries.length).toBe(3);
  expect(retries[2]).toEqual(retries[1]);
  expect(actions.filter((action) => action.method === "DELETE")).toHaveLength(
    1,
  );
  expect(actions[2]?.body).toMatchObject({
    action: "delete",
    acknowledge_data_loss: true,
    data_policy_version: policy.data_policy_version,
  });
  await page.getByRole("button", { name: "Close", exact: true }).click();
  const slot = page.getByRole("region", { name: "Personal free slot" });
  await expect(slot).toContainText("1 of 1 in use");
  await expect(page.getByRole("dialog")).toHaveCount(0);
  current = {
    ...current,
    resource_state: "deleted",
    cleanup_state: "complete",
  };
  operation = {
    ...operation,
    state: "succeeded",
    terminal_at: "2026-10-26T00:00:00Z",
    cleanup_state: "complete",
  } as typeof operation;
  await page.route("**/api/v1/remote-machine-entitlement**", (route) =>
    route.fulfill({
      json: {
        eligible: true,
        free_slot_available: true,
        machine_id: null,
        reason: null,
        policy_version: policy.policy_version,
        policy,
        platform: remoteMachinePlatformFixture,
      },
    }),
  );
  // Finality must refresh the mounted list and entitlement with Settings closed.
  // Reopening the retained detail happens only after that transition is proven.
  await expect(page.getByRole("article", { name: machine.name })).toHaveCount(
    0,
    { timeout: 15000 },
  );
  await expect(slot).toContainText("Available · 0 of 1");
  expect(retries).toHaveLength(3);
  expect(actions.filter((action) => action.method === "DELETE")).toHaveLength(
    1,
  );
  await page.goto(`/assistant/machines?machine=${machine.node_id}`);
  await expect(
    page.getByRole("link", { name: "Create another machine", exact: true }),
  ).toBeVisible();
  await expect(
    page.getByRole("form", { name: "Machine settings", exact: true }),
  ).toHaveCount(0);
  await expect(
    page.getByRole("button", { name: "Retry deletion", exact: true }),
  ).toHaveCount(0);
  await evidence(page, "production-deleted-390-dark");
});
