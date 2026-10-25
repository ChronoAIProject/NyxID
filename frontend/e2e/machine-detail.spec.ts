import { expect, test, type Locator, type Page } from "@playwright/test";
import { machineAccessFixture, machineContextReviewFixture } from "../src/test/machine-access-fixture";
import type { MachineContextReview } from "../src/schemas/machine-access";
import {
  agent,
  deleteOperation,
  emptyCapabilities,
  fixtureError,
  installMachineMock,
  machineId,
  operationId,
  otherMachineId,
  sourceGrant,
  sourceKey,
  type ApiWrite,
  type MachineMock,
} from "./machine-detail-fixtures";

const tabs = {
  overview: /^Overview$/i,
  access: /^Access(?: permissions)?$/i,
  jobs: /^(Jobs|Tasks)$/i,
  confirmations: /^(Pending )?Confirmations(?: \d+)?$/i,
  billing: /^(Billing|Costs)$/i,
  events: /^(Events|Timeline)$/i,
  settings: /^(Settings(?: and rules| & rules)?|Policy and setup)$/i,
};
const detail = (page: Page) =>
  page.getByRole("dialog", { name: "Remote machine details", exact: true });
async function show(page: Page) {
  await page.goto(`/assistant/machines?machine=${machineId}`);
  await expect(
    detail(page).getByRole("tab", { name: tabs.overview }),
  ).toBeVisible();
  return detail(page);
}
async function selectTab(panel: Locator, name: keyof typeof tabs) {
  await panel.getByRole("tab", { name: tabs[name] }).click();
  await expect(panel.getByRole("tab", { name: tabs[name] })).toHaveAttribute(
    "aria-selected",
    "true",
  );
}
async function noOverflow(page: Page) {
  const root = await page.evaluate(() => ({
    width: window.innerWidth,
    scroll: document.documentElement.scrollWidth,
  }));
  expect(root.scroll, "page must fit viewport").toBeLessThanOrEqual(
    root.width + 1,
  );
  for (const panel of await page.getByRole("dialog").all()) {
    if (await panel.isVisible()) {
      const bounds = await panel.evaluate((element) => ({
        client: element.clientWidth,
        scroll: element.scrollWidth,
      }));
      expect(
        bounds.scroll,
        "dialog content must fit dialog",
      ).toBeLessThanOrEqual(bounds.client + 1);
    }
  }
}
async function expandOperation(panel: Locator) {
  const disclosure = panel
    .locator("summary")
    .filter({ hasText: /^View operation progress$/i })
    .first();
  await expect(disclosure).toBeVisible();
  const operationDetails = disclosure.locator("..");
  if ((await operationDetails.getAttribute("open")) === null)
    await disclosure.click();
  await expect(operationDetails).toHaveAttribute("open", "");
  const diagnostics = operationDetails
    .locator("summary")
    .filter({ hasText: /^Diagnostic information$/i })
    .first();
  await expect(diagnostics).toBeVisible();
  const diagnosticDetails = diagnostics.locator("..");
  if ((await diagnosticDetails.getAttribute("open")) === null)
    await diagnostics.click();
  await expect(diagnosticDetails).toHaveAttribute("open", "");
  await expect(
    panel.getByText(operationId, { exact: true }).first(),
  ).toBeVisible();
}
async function deleting(page: Page) {
  const mock = await installMachineMock(page, { operation: deleteOperation() });
  mock.machine = {
    ...mock.machine,
    resource_state: "deleting",
    active_operation_id: operationId,
    cleanup_state: "pending",
  };
  return mock;
}
function record(route: Parameters<typeof fixtureError>[0], writes: ApiWrite[]) {
  const request = route.request();
  writes.push({
    path: new URL(request.url()).pathname.replace("/api/v1", ""),
    method: request.method(),
    body: request.postDataJSON(),
    key: request.headers()["idempotency-key"],
  });
}

for (const width of [1440, 320]) {
  test(`seven tabs, selection URL, back and settings-owned delete at ${width}px`, async ({
    page,
  }, info) => {
    await page.setViewportSize({ width, height: 1000 });
    const errors = new Set<string>();
    page.on("pageerror", (error) => errors.add(error.message));
    const mock = await installMachineMock(page);
    await page.goto("/assistant/machines");
    await page
      .getByRole("button", {
        name: width < 768 ? "View details" : `View ${mock.machine.name}`,
        exact: true,
      })
      .click();
    await expect(page).toHaveURL(new RegExp(`machine=${machineId}`));
    const panel = detail(page);
    await expect(panel.getByRole("tab")).toHaveCount(7);
    for (const name of Object.keys(tabs) as (keyof typeof tabs)[]) {
      await selectTab(panel, name);
      await expect(panel.getByRole("tabpanel")).toBeVisible();
      await noOverflow(page);
    }
    await selectTab(panel, "overview");
    await expect(
      panel.getByRole("button", { name: "Delete machine", exact: true }),
    ).toHaveCount(0);
    await selectTab(panel, "settings");
    await panel
      .getByRole("button", { name: "Delete machine", exact: true })
      .click();
    const review = page.getByRole("dialog", {
      name: /^Delete (machine|permanently)$/i,
    });
    await expect(review).toBeVisible();
    await expect(
      review.getByRole("button", {
        name: /Confirm delete|Delete machine|Delete permanently/i,
      }),
    ).toBeDisabled();
    await noOverflow(page);
    await review.getByRole("button", { name: "Cancel", exact: true }).click();
    expect(mock.writes).toEqual([]);
    await page.screenshot({
      path: info.outputPath(`settings-${width}.png`),
      fullPage: true,
      animations: "disabled",
    });
    const back = panel.getByRole("button", {
      name: "Back to machines",
      exact: true,
    });
    if (await back.count()) await back.click();
    else
      await panel.getByRole("button", { name: "Close", exact: true }).click();
    await expect(page).toHaveURL(/\/assistant\/machines$/);
    await expect(panel).toHaveCount(0);
    await show(page);
    await page.reload();
    await expect(
      detail(page).getByRole("heading", {
        name: mock.machine.name,
        exact: true,
      }),
    ).toBeVisible();
    expect([...errors]).toEqual([]);
    expect(mock.reads.some((path) => path.endsWith("/machine-settings"))).toBe(
      false,
    );
    expect(
      mock.reads.some((path) => /\/agents\/[^/]+\/grants/.test(path)),
    ).toBe(false);
  });

  test(`retry accepted stays deleting with actual counters and deadline diagnosis at ${width}px`, async ({
    page,
  }, info) => {
    await page.setViewportSize({ width, height: 1000 });
    const mock = await deleting(page);
    const requests: ApiWrite[] = [];
    let release!: () => void;
    const pending = new Promise<void>((resolve) => {
      release = resolve;
    });
    await page.route(
      `**/api/v1/remote-machine-operations/${operationId}/retry-delete`,
      async (route) => {
        record(route, requests);
        await pending;
        mock.operation = deleteOperation({
          retry_count: 154,
          delete_retry_attempts: 1,
          delete_retry_stopped: false,
          next_attempt_at: "2099-01-01T00:01:00Z",
        });
        mock.machine = { ...mock.machine, revision: 59 };
        await route.fulfill({
          status: 202,
          json: { operation: mock.operation, replayed: false },
        });
      },
    );
    const panel = await show(page);
    await expandOperation(panel);
    await expect(
      panel.getByText("delete_kubernetes_access_denied", { exact: true }),
    ).toBeVisible();
    await expect(
      panel
        .getByText(/exceeded|overdue|beyond.*target|target.*passed/i)
        .first(),
    ).toBeVisible();
    await expect(panel.getByText("10 / 10", { exact: true })).toBeVisible();
    await panel
      .getByRole("button", { name: "Retry deletion", exact: true })
      .click();
    await expect(
      panel.getByRole("button", { name: /Retrying deletion|Submitting/i }),
    ).toBeDisabled();
    await expect.poll(() => requests.length).toBe(1);
    expect(requests[0]?.body).toEqual({
      expected_revision: 58,
      expected_retry_count: 153,
    });
    expect(requests[0]?.key).toMatch(/^[0-9a-f-]{36}$/);
    release();
    await expect(panel.getByText("1 / 10", { exact: true })).toBeVisible();
    await expect(panel.getByText("154", { exact: true })).toBeVisible();
    await expect(
      panel.getByRole("button", { name: "Retry deletion", exact: true }),
    ).toHaveCount(0);
    await expect(
      panel.getByText(operationId, { exact: true }).first(),
    ).toBeVisible();
    await expect(
      panel.getByText("deleting", { exact: true }).first(),
    ).toBeVisible();
    await noOverflow(page);
    await page.screenshot({
      path: info.outputPath(`retry-${width}.png`),
      fullPage: true,
      animations: "disabled",
    });
  });

  test(`deleted supplemental cleanup retains terminal facts at ${width}px`, async ({
    page,
  }) => {
    await page.setViewportSize({ width, height: 1000 });
    const mock = await deleting(page);
    mock.machine = {
      ...mock.machine,
      resource_state: "deleted",
      authorization_state: "revoked",
      billing_state: "closed",
      active_operation_id: operationId,
    };
    mock.operation = deleteOperation({
      state: "succeeded",
      terminal_at: "2026-10-16T01:00:00Z",
    });
    mock.listed = false;
    const panel = await show(page);
    await expandOperation(panel);
    await expect(
      panel
        .getByText(/supplemental|follow.up.*cleanup|background.*cleanup/i)
        .first(),
    ).toBeVisible();
    await expect(
      panel.getByText("succeeded", { exact: true }).first(),
    ).toBeVisible();
    await expect(
      panel.getByRole("button", { name: "Retry deletion", exact: true }),
    ).toHaveCount(0);
    await selectTab(panel, "settings");
    const deletion = panel.getByRole("button", {
      name: "Delete machine",
      exact: true,
    });
    if (await deletion.count()) await expect(deletion).toBeDisabled();
    expect(mock.writes).toEqual([]);
    await noOverflow(page);
  });

  test(`owner desktop entry retains the existing route and private viewer at ${width}px`, async ({
    page,
  }) => {
    await page.setViewportSize({ width, height: 1000 });
    const mock = await installMachineMock(page);
    mock.machine.capabilities = ["shell", "files", "browser", "computer"];
    await page.route(
      "**/api/v1/assistant/nyxagent/machines/*/contexts",
      (route) => route.fulfill({ json: [] }),
    );
    const sockets: string[] = [];
    await page.routeWebSocket(
      "**/api/v1/assistant/nyxagent/machines/*/desktop",
      (socket) => {
        sockets.push(socket.url());
        socket.send(
          JSON.stringify({
            type: "error",
            error_code: 12417,
            message: "Fixture display unavailable",
          }),
        );
      },
    );
    const panel = await show(page);
    const desktop = panel.getByRole("link", {
      name: "Watch / Take over desktop",
      exact: true,
    });
    await expect(desktop).toHaveAttribute(
      "href",
      `/assistant/machines/${machineId}/desktop`,
    );
    await noOverflow(page);
    await desktop.click();
    await expect(page).toHaveURL(
      new RegExp(`/assistant/machines/${machineId}/desktop$`),
    );
    const viewer = page.getByRole("region", {
      name: "Live machine desktop",
      exact: true,
    });
    await expect(viewer).toHaveAttribute("data-private", "true");
    await expect(
      viewer.getByText("Fixture display unavailable", { exact: true }),
    ).toBeVisible();
    expect([...new Set(sockets)]).toEqual([
      `ws://localhost:4628/api/v1/assistant/nyxagent/machines/${machineId}/desktop`,
    ]);
    await noOverflow(page);
    await page.goBack();
    await expect(
      detail(page).getByRole("heading", {
        name: mock.machine.name,
        exact: true,
      }),
    ).toBeVisible();
    expect(mock.writes).toEqual([]);
  });
}

test("delayed detail loading and empty panels never fabricate data", async ({
  page,
}) => {
  const mock = await installMachineMock(page, { listed: false });
  let release!: () => void;
  const pending = new Promise<void>((resolve) => {
    release = resolve;
  });
  await page.route(`**/api/v1/remote-machines/${machineId}`, async (route) => {
    await pending;
    await route.fulfill({ json: { machine: mock.machine } });
  });
  await page.goto(`/assistant/machines?machine=${machineId}`);
  await expect(
    detail(page).getByRole("status", {
      name: "Loading remote machine",
      exact: true,
    }),
  ).toBeVisible();
  await expect(detail(page).getByRole("tab")).toHaveCount(0);
  release();
  const panel = detail(page);
  await expect(panel.getByRole("tab", { name: tabs.overview })).toBeVisible();
  await selectTab(panel, "jobs");
  await expect(panel.getByText(/No jobs match these filters/i)).toBeVisible();
  await selectTab(panel, "confirmations");
  await expect(
    panel.getByText(/No pending or approved confirmations/i),
  ).toBeVisible();
  await selectTab(panel, "access");
  await expect(panel.getByText(/No external grants/i)).toBeVisible();
  expect(mock.writes).toEqual([]);
});

test("retry unknown result reuses the reviewed request and idempotency key", async ({
  page,
}) => {
  await deleting(page);
  const requests: ApiWrite[] = [];
  await page.route(
    `**/api/v1/remote-machine-operations/${operationId}/retry-delete`,
    async (route) => {
      record(route, requests);
      await route.abort("connectionreset");
    },
  );
  const panel = await show(page);
  await expandOperation(panel);
  await panel
    .getByRole("button", { name: "Retry deletion", exact: true })
    .click();
  await expect(
    panel
      .getByText(/result.*unknown|outcome.*unknown|unknown.*result/i)
      .first(),
  ).toBeVisible();
  await panel
    .getByRole("button", {
      name: /Retry deletion|Confirm.*result|Check.*result/i,
    })
    .click();
  await expect.poll(() => requests.length).toBe(2);
  expect(requests[1]).toEqual(requests[0]);
  await expect(panel.getByText("153", { exact: true })).toBeVisible();
});

test("retry rejection preserves attempts; conflict refetches without automatic resubmission", async ({
  page,
}) => {
  const mock = await deleting(page);
  const requests: ApiWrite[] = [];
  await page.route(
    `**/api/v1/remote-machine-operations/${operationId}/retry-delete`,
    async (route) => {
      record(route, requests);
      if (requests.length === 1)
        return fixtureError(route, 403, "Fixture retry permission denied.");
      mock.machine = { ...mock.machine, revision: 60 };
      return fixtureError(
        route,
        409,
        "Fixture revision conflict; review the latest operation.",
      );
    },
  );
  const panel = await show(page);
  await expandOperation(panel);
  await panel
    .getByRole("button", { name: "Retry deletion", exact: true })
    .click();
  await expect(
    panel.getByText("Fixture retry permission denied.", { exact: true }),
  ).toBeVisible();
  await expect(panel.getByText("153", { exact: true })).toBeVisible();
  await panel
    .getByRole("button", { name: "Retry deletion", exact: true })
    .click();
  await expect(
    panel.getByText(/conflict|state changed/i).first(),
  ).toBeVisible();
  await expect
    .poll(
      () =>
        mock.reads.filter((path) => path === `/remote-machines/${machineId}`)
          .length,
    )
    .toBeGreaterThan(1);
  expect(requests).toHaveLength(2);
  await expect(
    panel.getByRole("button", {
      name: /review.*latest|reload.*review|review.*again|refresh.*review/i,
    }),
  ).toBeVisible();
  expect(
    requests.every(
      (request) =>
        (request.body as { expected_revision: number }).expected_revision ===
        58,
    ),
  ).toBe(true);
});

test("closed resume admission retains existing machine reading and reviewed delete", async ({
  page,
}) => {
  const mock = await installMachineMock(page, { admissions: false });
  mock.machine.resource_state = "paused";
  const panel = await show(page);
  await expect(
    panel.getByRole("button", { name: "Resume machine", exact: true }),
  ).toBeDisabled();
  await selectTab(panel, "settings");
  await expect(
    panel.getByRole("button", { name: "Delete machine", exact: true }),
  ).toBeEnabled();
  expect(mock.writes).toEqual([]);
});

for (const multiple of [true, false]) {
  test(`remove current source assignment ${multiple ? "preserves other machines" : "revokes last machine grant only"}`, async ({
    page,
  }) => {
    const grant = sourceGrant(multiple);
    const mock = await installMachineMock(page, { grant });
    const writes: ApiWrite[] = [];
    await page.route(
      "**/api/v1/api-keys/parent-key/machine-grant",
      async (route) => {
        if (route.request().method() === "GET") return route.fallback();
        record(route, writes);
        const body = route.request().postDataJSON();
        mock.grant = multiple
          ? {
              ...grant,
              revision: 8,
              assignments: {
                [otherMachineId]: grant.assignments[otherMachineId]!,
              },
            }
          : {
              ...grant,
              revision: 8,
              status: "revoked",
              revoked_at: "2026-10-21T01:00:00Z",
            };
        if (multiple)
          expect(Object.keys(body.selection.assignments)).toEqual([
            otherMachineId,
          ]);
        await route.fulfill({ json: mock.grant });
      },
    );
    const panel = await show(page);
    await selectTab(panel, "access");
    await panel
      .getByRole("button", { name: "Manage source permissions", exact: true })
      .click();
    await expect(
      panel.getByText("Current parent Key review", { exact: true }),
    ).toBeVisible();
    await expect
      .poll(() => mock.reads.includes("/api-keys/parent-key"))
      .toBe(true);
    await panel
      .getByRole("button", { name: "Remove current Machine", exact: true })
      .click();
    const review = page.getByRole("dialog", {
      name: multiple
        ? "Remove this Machine from the source grant?"
        : "Remove the last Machine and revoke this source grant?",
      exact: true,
    });
    await expect(
      review
        .getByText(
          multiple
            ? /other.*machine|remaining.*machine/i
            : /entire.*grant|whole.*grant|last.*machine/i,
        )
        .first(),
    ).toBeVisible();
    await expect(
      review
        .getByText(/parent.*key.*(remain|preserv|not)|independent.*service/i)
        .first(),
    ).toBeVisible();
    for (const checkbox of await review.getByRole("checkbox").all())
      await checkbox.check();
    expect(writes).toEqual([]);
    await review
      .getByRole("button", {
        name: multiple
          ? "Remove current Machine"
          : "Remove and revoke source grant",
        exact: true,
      })
      .click();
    await expect.poll(() => writes.length).toBe(1);
    expect(writes[0]?.method).toBe(multiple ? "PUT" : "DELETE");
    expect(writes[0]?.path).toBe("/api-keys/parent-key/machine-grant");
    expect(writes[0]?.body).toMatchObject({
      expected_revision: 7,
      expected_grant_id: grant.id,
    });
    if (multiple) {
      expect(writes[0]?.body).toMatchObject({
        expected_state_version: sourceKey.state_version,
      });
      expect((writes[0]?.body as { selection: unknown }).selection).toEqual({
        separation: "shared_legacy",
        legacy_warning_acknowledged: true,
        assignments: {
          [otherMachineId]: {
            capabilities: grant.assignments[otherMachineId]!.capabilities,
            file_roots: ["/other-workspace"],
            service_ids: grant.assignments[otherMachineId]!.service_ids,
          },
        },
        expires_at: grant.expires_at,
      });
    } else {
      expect(writes[0]?.body).toEqual({
        expected_revision: 7,
        expected_grant_id: grant.id,
      });
    }
    expect(mock.writes).toEqual([]);
    await noOverflow(page);
  });
}

test("assignment survives capability save failure and preserves unrelated grants", async ({
  page,
}) => {
  const mock = await installMachineMock(page, { agents: true });
  const writes: ApiWrite[] = [];
  await page.route(
    `**/api/v1/assistant/nyxagent/agents/${agent.id}/grants`,
    async (route) => {
      expect(route.request().method()).toBe("PUT");
      record(route, writes);
      mock.assigned = true;
      await route.fulfill({ json: {} });
    },
  );
  await page.route(
    `**/api/v1/assistant/nyxagent/agents/${agent.id}/machines/${machineId}`,
    async (route) => {
      record(route, writes);
      await fixtureError(route, 500, "Fixture capability save unavailable.");
    },
  );
  const panel = await show(page);
  await selectTab(panel, "access");
  await panel
    .getByRole("combobox", { name: "Agent for machine access" })
    .click();
  await page.getByRole("option", { name: agent.display_name }).click();
  const membership = panel.getByRole("form", {
    name: "Specialist machine membership",
  });
  await membership.getByRole("checkbox").check();
  await membership
    .getByRole("button", { name: "Save assignment", exact: true })
    .click();
  const capabilities = panel.getByRole("form", { name: /capabilities$/i });
  await capabilities
    .getByRole("checkbox", { name: "Shell commands", exact: true })
    .check();
  await capabilities
    .getByRole("button", { name: /^Save (agent )?capabilities$/ })
    .click();
  await expect(
    panel.getByRole("alert").filter({
      hasText:
        "Assignment succeeded, but capability changes failed. The assignment remains in place; reload the current capability state and retry editing when it is available.",
    }),
  ).toBeVisible();
  await expect(
    capabilities.getByRole("alert").filter({
      hasText: "Could not save capabilities. Reload if access changed.",
    }),
  ).toBeVisible();
  await expect(membership.getByRole("checkbox")).toBeChecked();
  expect(writes[0]?.body).toEqual({
    services: agent.services,
    account_read: true,
    machines: [otherMachineId, machineId],
    logins: agent.logins,
    guest_access: agent.guest_access,
  });
  expect(writes[1]?.body).toEqual({
    expected_revision: 8,
    capabilities: { ...emptyCapabilities, shell: true },
    saved_login_ids: ["saved-login-preserve"],
  });
  expect(writes).toHaveLength(2);
  expect(mock.writes).toEqual([]);
  expect(mock.reads.some((path) => /\/agents\/[^/]+\/grants/.test(path))).toBe(
    false,
  );
});

for (const width of [1440, 320]) {
  test(`context opt-in stays independent from capability editing and authority delivery at ${width}px`, async ({ page }, info) => {
    await page.setViewportSize({ width, height: 1000 });
    const mock = await installMachineMock(page, {
      agents: true,
      assigned: true,
      capabilityEditing: false,
      contextSetup: true,
    });
    const capabilities = { ...emptyCapabilities, files: true };
    await page.route(`**/api/v1/assistant/nyxagent/agents/${agent.id}/machines`, async route => {
      await route.fulfill({ json: [{
        ...machineAccessFixture,
        node_id: machineId,
        name: mock.machine.name,
        revision: 8,
        assignment_revision: 8,
        capability_edit_enabled: false,
        context_setup_enabled: true,
        machine_configuration: { ...mock.machine.machine_configuration, state: "applied", reason: null },
        protocol_v2: true,
        can_edit: true,
        legacy: false,
        mode: "shared_legacy",
        capabilities,
        ceiling: { shell: true, files: true, browser: true, computer: true, developer_browser: true },
        saved_login_ids: ["saved-login-preserve"],
        separated: { available: true, reason: null, landlock_abi: 6 },
        revocation_pending: true,
        execution_revocation_pending: false,
        delivery_pending: true,
        quarantine_pending: true,
      }] });
    });
    const requests: ApiWrite[] = [];
    const review = { ...machineContextReviewFixture, agent_id: agent.id, node_id: machineId, expected_revision: 8, capabilities, saved_login_ids: ["saved-login-preserve"] };
    await page.route(`**/api/v1/assistant/nyxagent/agents/${agent.id}/machines/${machineId}/context-prepare`, async route => {
      record(route, requests);
      await route.fulfill({ json: review });
    });
    await page.route(`**/api/v1/assistant/nyxagent/agents/${agent.id}/machines/${machineId}/context-reviews/review-1`, route => route.fulfill({ json: review }));
    const panel = await show(page);
    await selectTab(panel, "access");
    await panel.getByRole("combobox", { name: "Agent for machine access" }).click();
    await page.getByRole("option", { name: agent.display_name }).click();
    const form = panel.getByRole("form", { name: /capabilities$/i });
    await expect(form.getByText("Assigned", { exact: true })).toBeVisible();
    await expect(form.getByText("Saved tools: File tools", { exact: true })).toBeVisible();
    await expect(form.getByRole("checkbox", { name: "File tools", exact: true })).toBeChecked();
    await expect(form.getByRole("checkbox", { name: "File tools", exact: true })).toBeDisabled();
    await expect(form.getByRole("button", { name: "Save capabilities", exact: true })).toBeDisabled();
    await expect(form.getByText(/Earlier execution authority has expired/)).toBeVisible();
    await expect(form.getByText(/Waiting for machine acknowledgement/)).toBeVisible();
    await expect(form.getByText(/Browser cleanup is pending/)).toBeVisible();
    await expect(form.getByText(/Earlier commands have stopped/)).toHaveCount(0);
    await form.getByRole("button", { name: "Separate workspace and browser for this agent", exact: true }).click();
    await expect(form.getByText("Stored review review-1", { exact: true })).toBeVisible();
    await expect(form.getByText("Reviewed tools: File tools", { exact: true })).toBeVisible();
    await expect(form.getByRole("button", { name: "Approve and apply reviewed mode", exact: true })).toBeDisabled();
    expect(requests).toHaveLength(1);
    expect(requests[0]?.body).toEqual({ mode: "separated", policy_version: "machine-context-mode-v1", acknowledge_separation: true });
    expect(mock.writes).toEqual([]);
    await noOverflow(page);
    await page.screenshot({ path: info.outputPath(`agent-access-${width}.png`), fullPage: true });
  });
}

for (const width of [1440, 320]) {
  test(`owner stored review rejects stale confirmation and commits exact rereview at ${width}px`, async ({ page }, info) => {
    await page.setViewportSize({ width, height: 1000 });
    const mock = await installMachineMock(page, { agents: true, assigned: true, contextSetup: true });
    const config = { ...machineContextReviewFixture.machine_configuration!, state: "applied", reason: null };
    let revision = 8;
    let prepared = 0;
    let committed = false;
    let current: MachineContextReview = { ...machineContextReviewFixture, agent_id: agent.id, node_id: machineId, expected_revision: revision, machine_configuration: config };
    const writes: ApiWrite[] = [];
    const base = `/api/v1/assistant/nyxagent/agents/${agent.id}/machines/${machineId}`;
    await page.route(`**/api/v1/assistant/nyxagent/agents/${agent.id}/machines`, route => route.fulfill({ json: [{
      ...machineAccessFixture, node_id: machineId, name: mock.machine.name, revision,
      assignment_revision: committed ? 10 : revision, context_setup_enabled: true, capabilities: prepared > 1 ? emptyCapabilities : { ...emptyCapabilities, files: true },
      machine_configuration: committed ? { ...config, state: "pending", reason: "current_runtime_configuration_evidence_required" } : config,
      authorization_applied: committed, environment_applied: false,
    }] }));
    await page.route(`**${base}/context-prepare`, async route => {
      record(route, writes);
      prepared += 1;
      current = { ...current, review_id: `review-${prepared}`, expected_revision: revision, capabilities: prepared > 1 ? emptyCapabilities : { ...emptyCapabilities, files: true }, saved_login_ids: [], machine_configuration: config };
      await route.fulfill({ json: current });
    });
    await page.route(`**${base}/context-reviews/*`, route => route.fulfill({ json: current }));
    await page.route(`**${base}/context-reviews/*/commit`, async route => {
      record(route, writes);
      if (prepared === 1) {
        revision = 9;
        return fixtureError(route, 409, "machine_context_review_stale");
      }
      committed = true;
      revision = 10;
      current = { ...current, status: "committed", committed_revision: 10, applicable: false, authorization_applied: true, environment_applied: false,
        machine_configuration: { ...config, state: "pending", reason: "current_runtime_configuration_evidence_required" } };
      await route.fulfill({ json: current });
    });
    await page.goto(`/assistant/machines?machine=${machineId}&agent=${agent.id}`);
    const panel = detail(page);
    await expect(panel.getByRole("tab", { name: "Access", exact: true })).toHaveAttribute("aria-selected", "true");
    await expect(panel.getByRole("combobox", { name: "Agent for machine access" })).toHaveText(agent.display_name);
    await panel.getByRole("button", { name: "Separate workspace and browser for this agent", exact: true }).click();
    await expect(panel.getByText("Reviewed tools: File tools", { exact: true })).toBeVisible();
    await panel.getByRole("checkbox", { name: /I approve this exact/ }).check();
    await panel.getByRole("button", { name: "Approve and apply reviewed mode", exact: true }).click();
    await expect(panel.getByText(/Review is stale or no longer authorized/)).toBeVisible();
    await expect(panel.getByRole("button", { name: "Approve and apply reviewed mode", exact: true })).toBeDisabled();
    await panel.getByRole("button", { name: "Prepare and review again", exact: true }).click();
    await expect(panel.getByText("Stored review review-2", { exact: true })).toBeVisible();
    await expect(panel.getByText("Reviewed tools: None — all tools remain off", { exact: true })).toBeVisible();
    await expect(panel.getByRole("checkbox", { name: /I approve this exact/ })).not.toBeChecked();
    await panel.getByRole("checkbox", { name: /I approve this exact/ }).check();
    await panel.getByRole("button", { name: "Approve and apply reviewed mode", exact: true }).click();
    await expect(panel.getByText(/Authorization committed at revision 10/)).toBeVisible();
    await expect(panel.getByText("Hosted configuration pending", { exact: true }).first()).toBeVisible();
    await expect(panel.getByText("Environment mode applied", { exact: true })).toHaveCount(0);
    expect(writes.filter(write => write.path.endsWith("/commit")).map(write => write.body)).toEqual([{}, {}]);
    expect(writes.filter(write => write.path.endsWith("context-prepare")).map(write => write.body)).toEqual([
      { mode: "separated", policy_version: "machine-context-mode-v1", acknowledge_separation: true },
      { mode: "separated", policy_version: "machine-context-mode-v1", acknowledge_separation: true },
    ]);
    expect(mock.writes).toEqual([]);
    await noOverflow(page);
    await page.screenshot({ path: info.outputPath(`owner-review-${width}.png`), fullPage: true });
  });
}

test("Machine Ready with Agent support unavailable keeps fees and pause/delete distinct", async ({ page }) => {
  const mock = await installMachineMock(page, { agents: true, assigned: true });
  await page.route(`**/api/v1/assistant/nyxagent/agents/${agent.id}/machines`, route => route.fulfill({ json: [{
    ...machineAccessFixture, node_id: machineId, name: mock.machine.name,
    mode: "separated", availability_reason: "separated_context_unavailable", separated: { available: false, landlock_abi: 6, reason: "separated_probe_failed" },
  }] }));
  const panel = await show(page);
  await expect(panel.getByText("Machine Ready", { exact: true })).toBeVisible();
  await expect(panel.getByText(/An unavailable Agent does not pause this machine or stop runtime charges/)).toBeVisible();
  await expect(panel.getByRole("button", { name: "Pause machine", exact: true })).toBeVisible();
  await selectTab(panel, "settings");
  await expect(panel.getByRole("button", { name: "Delete machine", exact: true })).toBeVisible();
  await selectTab(panel, "access");
  await panel.getByRole("combobox", { name: "Agent for machine access" }).click();
  await page.getByRole("option", { name: agent.display_name }).click();
  await expect(panel.getByText(/Agent tools unavailable: separated support is unavailable/)).toBeVisible();
  await expect(panel.getByText("Environment mode applied", { exact: true })).toHaveCount(0);
  expect(mock.writes).toEqual([]);
});

for (const status of [403, 503]) {
  test(`detail read ${status} exposes no stale machine actions`, async ({
    page,
  }) => {
    await installMachineMock(page, { detailStatus: status, listed: false });
    await page.goto(`/assistant/machines?machine=${machineId}`);
    const panel = detail(page);
    await expect(
      panel.getByText(
        status === 403
          ? "This machine is unavailable to this account."
          : "Fixture machine read failed.",
        { exact: true },
      ),
    ).toBeVisible();
    await expect(
      panel.getByRole("button", { name: "Retry", exact: true }),
    ).toBeVisible();
    await expect(panel.getByRole("tab")).toHaveCount(0);
    await expect(
      panel.getByRole("button", { name: "Delete machine", exact: true }),
    ).toHaveCount(0);
    await expect(
      panel.getByRole("button", { name: "Retry deletion", exact: true }),
    ).toHaveCount(0);
  });
}

test("harness blocks unknown API reads, writes and external fetches locally", async ({
  page,
}) => {
  const mock: MachineMock = await installMachineMock(page);
  await page.goto("/assistant/machines");
  const result = await page.evaluate(async () => {
    const get = await fetch("/api/v1/never-configured");
    const deletion = await fetch("/api/v1/never-configured", {
      method: "DELETE",
    });
    let externalBlocked = false;
    try {
      await fetch("http://127.0.0.1:3001/api/v1/never-configured");
    } catch {
      externalBlocked = true;
    }
    return {
      readStatus: get.status,
      writeStatus: deletion.status,
      externalBlocked,
    };
  });
  expect(result).toEqual({
    readStatus: 404,
    writeStatus: 404,
    externalBlocked: true,
  });
  expect(mock.unknown).toContain("DELETE /never-configured");
});
