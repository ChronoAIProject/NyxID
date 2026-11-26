import { expect, test } from "@playwright/test";
import { managedBot, mockDashboard } from "./managed-onboarding-fixtures";
import type { GuestSetupConfiguration } from "../src/schemas/channel-guest-access";

const root = "**/api/v1/channel-bots/managed-whatsapp/guest-setup";
const pagePath = "/channel-bots/managed-whatsapp";

test("ordinary bot settings stay available with guest setup off", async ({ page }) => {
  await mockDashboard(page);
  const guestSetup = page.waitForResponse((response) => response.url().endsWith("/guest-setup"));
  await page.route(root, (route) => route.fulfill({ json: { available: false, enabled: false, bot_revision: 0 } }));
  await page.goto(pagePath);
  await guestSetup;
  await expect(page.getByRole("heading", { name: managedBot.label })).toBeVisible();
  await expect(page.getByRole("button", { name: "Verify Bot" })).toBeVisible();
  await expect(page.getByRole("heading", { name: "Guest experience" })).toHaveCount(0);
});

test("pilot bot resumes a draft and renders two guest decisions before applying", async ({ page }) => {
  await mockDashboard(page);
  await page.route("**/api/v1/channel-bots/managed-whatsapp/guest-skills", (route) =>
    route.fulfill({ json: { revision: 0, skills_revision: 0, available: [], attachments: [] } }));
  const settings = {
    revision: 0,
    policy_revision: 0,
    draft: null as GuestSetupConfiguration | null,
    applied: null as GuestSetupConfiguration | null,
    effective_rule_count: 0,
    verified_guests: [],
    verified_rooms: [],
    sheets_connections: [],
    sources: [{
      id: "personal_records",
      collection_id: "personal_records",
      actions: ["create", "read", "update", "delete"],
      fields: ["title", "body"],
    }],
  };
  const response = () => ({ available: true, enabled: true, bot_revision: 1, settings });
  await page.route(root, (route) => route.fulfill({ json: response() }));
  await page.route(root + "/draft", (route) => {
    const draft = route.request().postDataJSON();
    expect(draft.expected_revision).toBe(0);
    expect(draft.configuration.rules[0].operations.update).toBeUndefined();
    settings.draft = draft.configuration;
    settings.revision = 1;
    return route.fulfill({ json: response() });
  });
  await page.route(root + "/preview", (route) => {
    expect(route.request().postDataJSON().configuration).toEqual(settings.draft);
    return route.fulfill({ json: {
      guests_admitted: [true, true],
      rule_count: 1,
      checks: [{
        rule: 1,
        source_id: "personal_records",
        action: "read",
        guest_a: true,
        guest_b: true,
        guest_b_on_a: false,
        guest_a_in_group: false,
        resource: null,
        read_fields: ["title", "body"],
        write_fields: [],
      }],
    } });
  });
  await page.route(root + "/apply", (route) => {
    const input = route.request().postDataJSON();
    expect(input.expected_revision).toBe(1);
    expect(input.configuration).toEqual(settings.draft);
    settings.applied = settings.draft;
    settings.draft = null;
    settings.revision = 2;
    settings.policy_revision = 1;
    settings.effective_rule_count = 1;
    return route.fulfill({ json: response() });
  });

  await page.goto(pagePath);
  await expect(page.getByRole("heading", { name: "Guest experience" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Share Google Sheets data" })).toHaveCount(0);
  await page.getByLabel("Who can talk to this bot?").selectOption("channel_policy");
  await page.getByRole("button", { name: "Manage their own data" }).click();
  await page.getByRole("checkbox", { name: "Update", exact: true }).click();
  await page.getByRole("button", { name: "Save draft" }).click();
  await expect(page.getByText(/Draft saved/)).toBeVisible();

  await page.reload();
  await expect(page.getByText(/Draft saved/)).toBeVisible();
  await expect(page.getByRole("checkbox", { name: "Update", exact: true })).not.toBeChecked();
  await expect(page.getByRole("button", { name: "Apply guest setup" })).toBeDisabled();
  await page.getByRole("button", { name: "Preview two guests" }).click();
  const result = page.getByRole("row", { name: /Personal records · Read/ });
  await expect(result.getByRole("cell")).toHaveText(["Allowed", "Allowed", "Denied", "Denied"]);
  await page.getByRole("button", { name: "Apply guest setup" }).click();
  await expect(page.getByText(/Policy revision 1 applies/)).toBeVisible();
});

test("Sheets setup offers the available action and requires an exact shared room", async ({ page }) => {
  await mockDashboard(page);
  await page.route("**/api/v1/channel-bots/managed-whatsapp/guest-skills", (route) =>
    route.fulfill({ json: { revision: 0, skills_revision: 0, available: [], attachments: [] } }));
  const observation = { platform: "discord", last_seen_at: "2026-11-25T08:00:00Z" };
  await page.route(root, (route) => route.fulfill({ json: {
    available: true,
    enabled: true,
    bot_revision: 1,
    settings: {
      revision: 0,
      policy_revision: 0,
      draft: null,
      applied: null,
      effective_rule_count: 0,
      verified_guests: [{ ...observation, id: "guest-fixture-a" }],
      verified_rooms: [{ ...observation, id: "verified-event-room" }],
      sheets_connections: [{ id: "workspace-connection", label: "Event workspace", actions: ["create"] }],
      sources: [{ id: "google_sheets", collection_id: "values", actions: ["read", "create"], fields: [] }],
    },
  } }));
  await page.route(root + "/preview", (route) => {
    const input = route.request().postDataJSON();
    expect(input.configuration.rules[0]).toEqual({
      source_id: "google_sheets",
      collection_id: "values",
      audience: {
        kind: "verified_audience",
        audience_id: "Event",
        guest_ids: ["guest-fixture-a"],
        room_id: "verified-event-room",
      },
      scope: { kind: "resources", record_ids: ["workspace-connection/event-workbook/Questions"] },
      delivery: "shared",
      operations: { create: { read: [], write: [] } },
    });
    return route.fulfill({ json: {
      guests_admitted: [true, true],
      rule_count: 1,
      checks: [{
        rule: 1, source_id: "google_sheets", action: "create",
        guest_a: false, guest_b: false, guest_b_on_a: null, guest_a_in_group: true,
        resource: "Questions", read_fields: [], write_fields: [],
      }],
    } });
  });
  await page.goto(pagePath);
  await page.getByLabel("Who can talk to this bot?").selectOption("channel_policy");
  await page.getByRole("button", { name: "Share Google Sheets data" }).click();
  await expect(page.getByLabel("Google Sheets connection")).toHaveValue("workspace-connection");
  await expect(page.getByLabel("What can guests do?")).toHaveValue("create");
  await expect(page.getByRole("option", { name: "Read an exact cell range" })).toHaveCount(0);
  await expect(page.getByRole("option", { name: "One verified guest" })).toHaveCount(0);
  await page.getByLabel("Who can use this rule?").selectOption("verified_audience");
  await page.getByLabel("Audience name").fill("Event");
  await page.getByRole("checkbox", { name: /Discord guest/ }).click();
  await page.getByLabel("Verified room").selectOption("verified-event-room");
  await page.getByLabel("Spreadsheet ID").fill("event-workbook");
  await page.getByLabel("Exact tab name").fill("Questions");
  await page.getByRole("button", { name: "Preview two guests" }).click();
  await expect(page.getByRole("row", { name: /Google Sheets · Create Questions/ }).getByRole("cell")).toHaveText([
    "Denied", "Denied", "—", "Allowed",
  ]);
});


test("an existing bot can preview a saved migration draft before opting in", async ({ page }) => {
  await mockDashboard(page);
  await page.route("**/api/v1/channel-bots/managed-whatsapp/guest-skills", (route) =>
    route.fulfill({ json: { revision: 0, skills_revision: 0, available: [], attachments: [] } }));
  let enabled = false;
  const settings = {
    revision: 0,
    policy_revision: 0,
    prepared_draft_revision: null as number | null,
    draft: null as GuestSetupConfiguration | null,
    applied: null as GuestSetupConfiguration | null,
    effective_rule_count: 0,
    verified_guests: [],
    verified_rooms: [],
    sheets_connections: [],
    sources: [],
  };
  const state = () => ({ available: enabled, enabled, bot_revision: enabled ? 1 : 0 });
  await page.route(root, async (route) => {
    if (route.request().method() === "PUT") {
      expect(route.request().postDataJSON()).toEqual({ enabled: true, expected_bot_revision: 0 });
      expect(settings.prepared_draft_revision).toBe(1);
      enabled = true;
      settings.applied = settings.draft;
      settings.draft = null;
      settings.prepared_draft_revision = null;
      settings.policy_revision = 1;
      return route.fulfill({ json: { ...state(), settings } });
    }
    return route.fulfill({ json: state() });
  });
  await page.route(root + "/staging", (route) => route.fulfill({ json: { ...state(), settings } }));
  await page.route(root + "/draft", (route) => {
    const request = route.request().postDataJSON();
    expect(request).toMatchObject({ expected_revision: 0, staging: true });
    settings.draft = request.configuration;
    settings.revision = 1;
    return route.fulfill({ json: { ...state(), settings } });
  });
  await page.route(root + "/preview", (route) => {
    expect(route.request().postDataJSON()).toEqual({ configuration: settings.draft, staging: true });
    return route.fulfill({ json: { guests_admitted: [false, false], rule_count: 0, checks: [] } });
  });
  await page.route(root + "/prepare", (route) => {
    expect(route.request().postDataJSON()).toEqual({ expected_revision: 1 });
    settings.revision = 2;
    settings.prepared_draft_revision = 1;
    return route.fulfill({ json: { ...state(), settings } });
  });

  await page.goto(pagePath);
  const toggle = page.getByRole("switch", { name: "Use guest access v1" });
  await expect(toggle).not.toBeChecked();
  await page.getByRole("button", { name: "Prepare guest access" }).click();
  await page.getByLabel("Who can talk to this bot?").selectOption("owner_only");
  await page.getByRole("button", { name: "Save draft" }).click();
  await expect(page.getByText(/Draft saved/)).toBeVisible();
  await page.getByRole("button", { name: "Preview two guests" }).click();
  await expect(page.getByText("No guest data grants.")).toBeVisible();
  await page.getByRole("button", { name: "Mark draft for activation" }).click();
  await expect(page.getByText("Draft revision 1 is marked for activation.")).toBeVisible();
  await expect(toggle).not.toBeChecked();
  await toggle.click();
  await expect(toggle).toBeChecked();
  await expect(page.getByText("Guest access v1 is active for this bot.")).toBeVisible();
});
