import { expect, test, type Page } from "@playwright/test";
import { mockDashboard } from "./managed-onboarding-fixtures";
import { billingWallet } from "../src/test/billing-fixture";
import type { RemoteMachine } from "../src/schemas/machines";
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
  await page.route("**/api/v1/assistant/agents**", (route) =>
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
          name: width < 768 ? "View details" : `View ${machine.name}`,
          exact: true,
        })
        .click();
      const detail = page.getByRole("dialog");
      const desktop = detail.getByRole("link", {
        name: "Watch / Take over desktop",
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

async function noHorizontalOverflow(page: Page) {
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= window.innerWidth,
    ),
  ).toBe(true);
  const dialog = page.getByRole("dialog");
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
        .getByRole("button", { name: "View details", exact: true })
        .click();
    else
      await page
        .getByRole("button", {
          name: `View ${machine.name}`,
          exact: true,
        })
        .click();
    const detail = page.getByRole("dialog");
    await expect(detail.getByText("WSS status unknown")).toBeVisible();
    await expect(
      detail.getByText(/No usage has been reported by the current runtime/),
    ).toBeVisible();
    await noHorizontalOverflow(page);
    await page.screenshot({
      path: testInfo.outputPath("machine-overview.png"),
      fullPage: true,
      animations: "disabled",
    });
    await detail.getByRole("tab", { name: "Jobs", exact: true }).click();
    await expect(
      detail.getByText(/Job history is unavailable on this server/),
    ).toBeVisible();
    await noHorizontalOverflow(page);
    await detail.getByRole("tab", { name: "Billing", exact: true }).click();
    await expect(
      detail.getByText(/Runtime billing details are unavailable/),
    ).toBeVisible();
    await expect(
      detail.getByText(/Historical contract lookup is unavailable/),
    ).toBeVisible();
    await noHorizontalOverflow(page);
    expect(errors).toEqual([]);
  });

  test(`free creation requires policy review and a safe durable request at ${width}px`, async ({
    page,
  }, testInfo) => {
    await page.setViewportSize({ width, height: 1000 });
    const writes = await machineApi(page, true);
    await page.goto("/assistant/machines");
    await page
      .getByRole("button", { name: "Create remote machine", exact: true })
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
    await expect(
      create.getByText(
        /Temporary storage is managed by the deployment environment/,
      ),
    ).toBeVisible();
    await expect(create.getByText(/2 GiB temporary storage/)).toHaveCount(0);
    await create.getByLabel("Machine name").fill("remote-research");
    await create.getByRole("checkbox", { name: /^Browser/ }).check();
    await expect(create.getByText("Eligible", { exact: true })).toBeVisible();
    await expect(
      create.getByText(/Paid creation requires a live synchronized quote/),
    ).toBeVisible();
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
    await create
      .getByRole("checkbox", { name: /I accept the shared environment/ })
      .check();
    await page.screenshot({
      path: testInfo.outputPath("machine-create-reviewed.png"),
      fullPage: true,
      animations: "disabled",
    });
    await submit.click();
    await expect.poll(() => writes.length).toBe(1);
    expect(writes[0]?.body).toEqual({
      name: "remote-research",
      capabilities: ["shell", "files", "browser"],
      specialist_id: null,
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
      new RegExp(
        `/assistant/machines\\?machine=${machine.node_id}&operation=${remoteMachineOperationFixture.id}`,
      ),
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
    await operation.getByRole("button", { name: "Close", exact: true }).click();
    await expect(
      page.getByRole("heading", { name: "Remote machine details" }),
    ).toBeVisible();
    await expect(
      page.getByRole("heading", { name: "remote-research", exact: true }),
    ).toBeVisible();
    await expect(
      page.getByText("No fixed capacity declared", { exact: true }),
    ).toBeVisible();
    await noHorizontalOverflow(page);
  });
}

// These fixtures follow machine_external_execution.rs and machine_external_service.rs.
// All requests remain intercepted; no live Node output or execution is exercised.
for (const width of [1440, 390]) {
  test(`owner output is transient and cancellation is not a physical stop at ${width}px`, async ({
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
    const job = {
      id: "job-live",
      job_id: "job-live",
      node_id: machine.node_id,
      runtime_id: "runtime-live",
      runtime_generation: 1,
      state: "running",
      created_at: "2026-10-13T20:00:00Z",
      expires_at: "2099-10-13T21:00:00Z",
      finished_at: null,
      exit_code: null,
      stop_reason: null,
      duration_ms: null,
      cancellation_requested_by: null,
      cancellation_requested_at: null,
      retain_until: null,
      binding: {
        kind: "external",
        principal,
        grant_id: "grant-live",
        grant_revision: 7,
      },
      services: [],
      output_persisted: false,
      output_query_available: true,
      output_availability: "query_required",
      cancellation_available: true,
    };
    const reads: { headers: Record<string, string>; search: string }[] = [];
    const cancellations: unknown[] = [];
    await page.route(
      `**/api/v1/remote-machines/${machine.node_id}/jobs**`,
      async (route) => {
        const url = new URL(route.request().url());
        if (url.pathname.endsWith("/output")) {
          reads.push({
            headers: route.request().headers(),
            search: url.search,
          });
          return route.fulfill({
            headers: { "Cache-Control": "no-store" },
            json:
              reads.length === 1
                ? {
                    job_id: job.id,
                    status: "running",
                    exit_code: null,
                    stdout: "Transient fixture output 9f341",
                    stderr: "",
                    stdout_bytes: 30,
                    stderr_bytes: 0,
                    output_offset: 30,
                    stderr_offset: 0,
                    truncated: false,
                    output_available: true,
                    duration_ms: 120,
                  }
                : {
                    job_id: job.id,
                    status: "running",
                    stdout: "",
                    stderr: "",
                    output_available: false,
                    output_unavailable_reason: "cache_evicted",
                  },
          });
        }
        if (url.pathname.endsWith("/cancel")) {
          expect(route.request().method()).toBe("POST");
          cancellations.push(route.request().postDataJSON());
          return route.fulfill({
            json: {
              job_id: job.id,
              status: "cancelled",
              cancellation_acknowledged: true,
              stop_acknowledged: false,
            },
          });
        }
        return route.fulfill({
          json: {
            jobs: [job],
            next_cursor: null,
            has_more: false,
            output_persisted: false,
          },
        });
      },
    );
    await page.goto("/assistant/machines");
    await page
      .getByRole("button", {
        name: width < 768 ? "View details" : `View ${machine.name}`,
        exact: true,
      })
      .click();
    const detail = page.getByRole("dialog");
    await detail.getByRole("tab", { name: "Jobs", exact: true }).click();
    await detail.getByRole("button", { name: "View job", exact: true }).click();
    await expect(
      detail.getByText("query_required", { exact: true }).last(),
    ).toBeVisible();
    expect(reads).toHaveLength(0);
    await detail
      .getByRole("button", { name: "Read live output", exact: true })
      .click();
    await expect(
      detail.getByText("Transient fixture output 9f341", { exact: true }),
    ).toBeVisible();
    expect(reads[0]?.headers["cache-control"]).toBe("no-store");
    expect(reads[0]?.headers.pragma).toBe("no-cache");
    expect(
      await page.evaluate(() => JSON.stringify([localStorage, sessionStorage])),
    ).not.toContain("Transient fixture output 9f341");
    await noHorizontalOverflow(page);
    await page.screenshot({
      path: testInfo.outputPath("machine-live-output.png"),
      fullPage: true,
      animations: "disabled",
    });
    await detail
      .getByRole("button", { name: "Next output page", exact: true })
      .click();
    await expect(
      detail.getByText(/Output unavailable: cache evicted/),
    ).toBeVisible();
    expect(reads[1]?.search).toBe("?output_offset=30&stderr_offset=0");
    await expect(
      detail.getByText("Transient fixture output 9f341", { exact: true }),
    ).toHaveCount(0);
    await detail
      .getByRole("button", { name: "Cancel job", exact: true })
      .click();
    await page
      .getByRole("button", { name: "Confirm cancellation", exact: true })
      .click();
    await expect(
      page.getByText(
        /Cancellation request acknowledged. Physical stop is not confirmed/,
      ),
    ).toBeVisible();
    expect(cancellations).toEqual([{}]);
    await noHorizontalOverflow(page);
    await page.screenshot({
      path: testInfo.outputPath("machine-cancellation-ack.png"),
      fullPage: true,
      animations: "disabled",
    });
  });

  test(`owner reviews global confirmation and sends the bound decision at ${width}px`, async ({
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
    await page.goto("/assistant/machines");
    await page
      .getByRole("button", {
        name: width < 768 ? "View details" : `View ${machine.name}`,
        exact: true,
      })
      .click();
    const detail = page.getByRole("dialog");
    await detail
      .getByRole("tab", { name: "Confirmations", exact: true })
      .click();
    await detail
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
    await page
      .getByRole("tab", { name: "Settings and rules", exact: true })
      .click();
    await expect(
      page.getByRole("button", { name: "Delete machine", exact: true }),
    ).toBeEnabled();
    expect(admissions).toEqual([]);
  });
}
