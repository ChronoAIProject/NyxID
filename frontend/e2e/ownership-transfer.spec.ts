import { expect, test } from "@playwright/test";
import { mockDashboard, managedBot } from "./managed-onboarding-fixtures";
import type { KeyInfo } from "../src/types/keys";

const person = {
  id: "22222222-2222-4222-8222-222222222222",
  display_name: "Calvin",
  email: "calvin@example.test",
  is_active: true,
};
const orgId = "33333333-3333-4333-8333-333333333333";
const connectedService: KeyInfo = {
  id: "connected-service",
  label: "Team API connection",
  slug: "team-api-connection",
  endpoint_url: "https://api.example.test",
  endpoint_id: "team-endpoint",
  api_key_id: "team-credential",
  credential_type: "api_key",
  auth_method: "bearer",
  auth_key_name: "Authorization",
  status: "active",
  catalog_service_id: "custom-service",
  catalog_service_slug: "team-api",
  catalog_service_name: "Team API",
  node_id: null,
  node_priority: 0,
  is_active: true,
  custom_user_agent: null,
  default_request_headers: null,
  ws_frame_injections: [],
  auto_connected: false,
  source_app_id: null,
  source_app_name: null,
  expires_at: null,
  last_used_at: null,
  error_message: null,
  created_at: managedBot.created_at,
  service_type: "http",
  ssh_host: null,
  ssh_port: null,
  ssh_ca_public_key: null,
  ssh_allowed_principals: null,
  ssh_certificate_ttl_minutes: null,
  openapi_spec_url: null,
  credential_source: { type: "org", org_id: orgId, org_name: "Team", role: "admin", allowed: true },
  permission_setup_url: null,
  permission_setup_scopes: null,
  authorship: { created_by: null, last_change: null },
};

for (const width of [1440, 390]) {
  test(`transfer an OAuth bot from its detail card at ${width}px`, async ({
    page,
  }) => {
    await page.setViewportSize({ width, height: 1000 });
    await mockDashboard(page);
    if (width === 390) {
      await page.route("**/api/v1/users/me", (route) =>
        route.fulfill({
          json: {
            id: "test-user",
            email: "owner@example.test",
            display_name: "Org admin",
            is_admin: false,
            role: "user",
            is_active: true,
            email_verified: true,
            created_at: managedBot.created_at,
          },
        }),
      );
    }
    await page.route("**/api/v1/ownership/*/*/authorization", (route) =>
      route.fulfill({ json: { can_transfer: true } }),
    );
    const bot = {
      ...managedBot,
      id: "connected-x",
      platform: "x",
      label: "Team X bot",
      status: "active",
      credential_source: "connection",
      user_id: orgId,
      connection_id: "connection",
      managed_setup: null,
    };
    await page.route("**/api/v1/channel-bots/connected-x", (route) =>
      route.fulfill({ json: bot }),
    );
    await page.route("**/api/v1/orgs", (route) =>
      route.fulfill({
        json: {
          orgs: [{ id: orgId, display_name: "Team", your_role: "admin" }],
        },
      }),
    );
    await page.route("**/api/v1/ownership/*/*/destinations?**", (route) =>
      route.fulfill({ json: { users: [person], total: 1 } }),
    );
    const writes: unknown[] = [];
    await page.route(
      "**/api/v1/ownership/channel_bot/connected-x/preview",
      (route) =>
        route.fulfill({
          json: {
            resource_kind: "channel_bot",
            resource_id: bot.id,
            name: bot.label,
            previous_owner_user_id: orgId,
            previous_owner_name: "Team",
            new_owner_user_id: person.id,
            destination_name: person.display_name,
            destination_type: "person",
            version: "a".repeat(64),
            routes_to_retire: 2,
            blockers: [],
            effects: [
              "The bot's dedicated X connection moves to the new owner. The connected X account and existing token copies are unchanged.",
              "Existing routes are permanently retired. The destination must assign its own agents before messages can be delivered.",
              "Historical conversations and message metadata stay with the previous owner. Already-dispatched external work may finish.",
            ],
          },
        }),
    );
    await page.route(
      "**/api/v1/ownership/channel_bot/connected-x/transfer",
      async (route) => {
        writes.push(route.request().postDataJSON());
        await route.fulfill({ json: { transfer_id: "receipt" } });
      },
    );
    await page.goto("/channel-bots/connected-x");
    await expect(
      page.getByRole("heading", { name: "Ownership transfer", exact: true }),
    ).toBeVisible();
    await page
      .getByRole("heading", { name: "Ownership transfer", exact: true })
      .locator("../..")
      .screenshot({ path: `/tmp/nyx-ownership-card-${width}.png` });
    await expect(
      page.getByRole("link", { name: "Ownership transfers", exact: true }),
    ).toHaveCount(width === 390 ? 0 : 1);
    await page
      .getByRole("button", { name: "Transfer ownership", exact: true })
      .click();
    const dialog = page.getByRole("dialog", {
      name: /Transfer ownership|Review ownership transfer/,
    });
    await dialog.getByRole("combobox", { name: "Destination type" }).click();
    await page.getByRole("option", { name: "Person", exact: true }).click();
    await dialog.getByRole("combobox", { name: "Destination owner" }).click();
    await page.getByRole("combobox", { name: "Search owners" }).fill("calvin");
    await expect(page.getByRole("option", { name: /Calvin/ })).toBeVisible();
    await page.screenshot({ path: `/tmp/nyx-ownership-picker-${width}.png` });
    await page.getByRole("option", { name: /Calvin/ }).click();
    await expect(
      dialog.getByRole("combobox", { name: "Destination owner" }),
    ).toContainText("Calvin");
    await expect(
      dialog.getByText("Find destination", { exact: true }),
    ).toHaveCount(0);
    expect(writes).toEqual([]);
    await dialog.screenshot({
      path: `/tmp/nyx-ownership-destination-${width}.png`,
    });
    await dialog
      .getByRole("button", { name: "Review transfer", exact: true })
      .click();
    await expect(
      dialog.getByText(
        "The bot's dedicated X connection moves to the new owner. The connected X account and existing token copies are unchanged.",
      ),
    ).toBeVisible();
    await expect(
      dialog.getByText("2 routes will be permanently retired."),
    ).toBeVisible();
    expect(
      await dialog.evaluate(
        (element) => element.scrollWidth <= element.clientWidth,
      ),
    ).toBe(true);
    await dialog.screenshot({ path: `/tmp/nyx-ownership-review-${width}.png` });
    await dialog
      .getByRole("button", { name: "Confirm transfer", exact: true })
      .click();
    await expect(page).toHaveURL(/\/channel-bots$/);
    expect(writes).toEqual([
      {
        new_owner_user_id: person.id,
        expected_version: "a".repeat(64),
        request_id: expect.any(String),
      },
    ]);
  });
}

for (const [connectedDetail, width] of [[false, 1440], [true, 1440], [true, 390]] as const) {
  test(`transfer a catalog service from ${connectedDetail ? "the existing Advanced tab" : "admin detail"} at ${width}px`, async ({
    page,
  }) => {
    await page.setViewportSize({ width, height: 1000 });
    await mockDashboard(page);
    if (connectedDetail) {
      await page.route("**/api/v1/users/me", (route) =>
        route.fulfill({
          json: {
            id: "test-user",
            email: "owner@example.test",
            display_name: "Owner",
            is_admin: false,
            role: "user",
            is_active: true,
            email_verified: true,
            created_at: managedBot.created_at,
          },
        }),
      );
    }
    await page.route("**/api/v1/ownership/*/*/authorization", (route) =>
      route.fulfill({ json: { can_transfer: true } }),
    );
    let service = {
      id: "custom-service",
      name: "Team API",
      slug: "team-api",
      description: "Shared team API",
      base_url: "https://api.example.test",
      service_type: "http",
      visibility: "private",
      auth_method: "none",
      auth_type: "none",
      auth_key_name: "",
      is_active: true,
      oauth_client_id: null,
      api_spec_url: null,
      service_category: "connection",
      requires_user_credential: false,
      created_by: "original-creator",
      owner_user_id: orgId,
      created_at: managedBot.created_at,
      updated_at: managedBot.updated_at,
      identity_propagation_mode: "none",
      anonymous_endpoints: [],
      your_binding_count: 0,
    };
    await page.route(
      "**/api/v1/ownership/service/custom-service/authorization",
      (route) =>
        route.fulfill({
          json: {
            can_transfer: !connectedDetail || service.owner_user_id === orgId,
            resource: connectedDetail && service.owner_user_id !== orgId ? null : {
              id: service.id,
              name: service.name,
              owner_user_id: service.owner_user_id,
              slug: service.slug,
              platform: null,
            },
          },
        }),
    );
    await page.route("**/api/v1/keys/connected-service", (route) =>
      route.fulfill({ json: connectedService }),
    );
    await page.route("**/api/v1/catalog/team-api", (route) =>
      route.fulfill({ json: service }),
    );
    await page.route("**/api/v1/catalog", (route) =>
      route.fulfill({ json: { entries: [service] } }),
    );
    await page.route("**/api/v1/providers", (route) =>
      route.fulfill({ json: { providers: [] } }),
    );
    await page.route("**/api/v1/providers/my-tokens", (route) =>
      route.fulfill({ json: { tokens: [] } }),
    );
    await page.route(
      "**/api/v1/services/custom-service/anonymous-endpoints",
      (route) => route.fulfill({ json: { endpoints: [] } }),
    );
    const reads: string[] = [];
    await page.route("**/api/v1/services/custom-service", async (route) => {
      reads.push(service.owner_user_id);
      await route.fulfill({ json: service });
    });
    await page.route("**/api/v1/ownership/*/*/destinations?**", (route) =>
      route.fulfill({
        json: {
          users: new URL(route.request().url()).searchParams.get("user_type") === "org"
            ? [{ ...person, id: orgId, display_name: "Current team" }]
            : [person],
          total: 1,
        },
      }),
    );
    await page.route("**/api/v1/services/custom-service/endpoints", (route) =>
      route.fulfill({ json: { endpoints: [] } }),
    );
    await page.route(
      "**/api/v1/services/custom-service/requirements",
      (route) => route.fulfill({ json: { requirements: [] } }),
    );
    await page.route(
      "**/api/v1/ownership/service/custom-service/preview",
      (route) =>
        route.fulfill({
          json: {
            resource_kind: "service",
            resource_id: service.id,
            name: service.name,
            previous_owner_user_id: orgId,
            previous_owner_name: "Team",
            new_owner_user_id: person.id,
            destination_name: person.display_name,
            destination_type: "person",
            version: "b".repeat(64),
            routes_to_retire: 0,
            blockers: [],
            effects: [
              "Shared service configuration remains managed by NyxID admins.",
            ],
          },
        }),
    );
    const writes: unknown[] = [];
    await page.route(
      "**/api/v1/ownership/service/custom-service/transfer",
      async (route) => {
        writes.push(route.request().postDataJSON());
        service = { ...service, owner_user_id: person.id };
        await route.fulfill({ json: { transfer_id: "receipt" } });
      },
    );
    await page.goto(
      connectedDetail
        ? "/keys/connected-service"
        : "/services/custom-service",
    );
    if (connectedDetail) {
      await expect(page.getByRole("tabpanel", { name: "Overview" })).toBeVisible();
      await expect(page.getByRole("button", { name: "Transfer ownership", exact: true })).toHaveCount(0);
      await expect(page.getByRole("tab", { name: "History", exact: true })).toBeVisible();
      await page.getByRole("tab", { name: "Advanced", exact: true }).click();
      await expect(page.getByText("Transfer the catalog definition for Team API to another person or organization.")).toBeVisible();
      await page.screenshot({ path: `/tmp/nyx-service-tabs-${width}.png`, animations: "disabled" });
      await page.getByRole("button", { name: "Transfer ownership", exact: true }).scrollIntoViewIfNeeded();
      await page.screenshot({ path: `/tmp/nyx-service-advanced-${width}.png`, fullPage: true });
    }
    await page
      .getByRole("button", { name: "Transfer ownership", exact: true })
      .click();
    const dialog = page.getByRole("dialog", {
      name: /Transfer ownership|Review ownership transfer/,
    });
    await dialog.getByRole("combobox", { name: "Destination owner" }).click();
    await expect(
      page.getByRole("option", { name: /Current team/ }),
    ).toHaveAttribute("aria-disabled", "true");
    await page.keyboard.press("Escape");
    await dialog.getByRole("combobox", { name: "Destination type" }).click();
    await page.getByRole("option", { name: "Person", exact: true }).click();
    await dialog.getByRole("combobox", { name: "Destination owner" }).click();
    await page.getByRole("option", { name: /^Calvin/ }).click();
    await dialog
      .getByRole("button", { name: "Review transfer", exact: true })
      .click();
    await dialog
      .getByRole("button", { name: "Confirm transfer", exact: true })
      .click();
    await expect(dialog).toHaveCount(0);
    if (!connectedDetail) await expect.poll(() => reads).toContain(person.id);
    else await expect(page.getByRole("button", { name: "Transfer ownership", exact: true })).toHaveCount(0);
    expect(writes).toEqual([
      {
        new_owner_user_id: person.id,
        expected_version: "b".repeat(64),
        request_id: expect.any(String),
      },
    ]);
    await expect(page).toHaveURL(
      connectedDetail ? /\/keys\/connected-service$/ : /\/services\/custom-service$/,
    );
  });
}
