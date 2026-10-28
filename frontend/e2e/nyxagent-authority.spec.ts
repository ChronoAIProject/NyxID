import { test, expect, type Page } from "@playwright/test";
import { openAccountMenu, openAssistant, sendMessage, composerInput, stopButton } from "./helpers";
import { mockDashboard } from "./managed-onboarding-fixtures";

const unexpectedRequests = new WeakMap<Page, string[]>();

test.beforeEach(async ({ page }) => {
  const unexpected: string[] = [];
  unexpectedRequests.set(page, unexpected);
  await page.route("**/api/v1/**", (route) => {
    unexpected.push(
      `${route.request().method()} ${new URL(route.request().url()).pathname}`,
    );
    return route.fulfill({
      status: 500,
      json: { message: "Unexpected request outside fixtures" },
    });
  });
  await page.route("**/api/v1/approvals/requests?**", (route) =>
    route.fulfill({ json: { requests: [], total: 0 } }),
  );
  await page.route("**/api/v1/users/me", (route) =>
    route.fulfill({
      json: {
        id: "d4f5a6b7-c8d9-4e0f-a1b2-c3d4e5f60718",
        email: "reader@example.com",
        display_name: "Reader",
        email_verified: true,
        mfa_enabled: false,
        is_admin: false,
        is_active: true,
        created_at: "2026-01-01T00:00:00Z",
        capabilities: {
          billing_available: true,
          enabled_features: ["assistant:nyxagent-engine"],
        },
      },
    }),
  );
  await page.route("**/api/v1/public/config", (route) =>
    route.fulfill({ json: { mcp_url: "https://nyxid.test/mcp" } }),
  );
  await page.route("**/api/v1/platform-personal-bindings", (route) =>
    route.fulfill({ json: { bindings: [], next_before_id: null } }),
  );
  await page.route("**/api/v1/onboarding/bindings", (route) =>
    route.fulfill({ json: { bindings: [] } }),
  );
  await page.route("**/api/v1/sessions", (route) =>
    route.fulfill({ json: [] }),
  );
  await page.route("**/api/v1/assistant/nyxagent/machines?**", (route) =>
    route.fulfill({ json: [] }),
  );
  await page.route("**/api/v1/saved-logins**", (route) =>
    route.fulfill({ json: [] }),
  );
  await page.route("**/api/v1/triggers", (route) =>
    route.fulfill({ json: { triggers: [] } }),
  );
  await page.route("**/api/v1/orgs", (route) =>
    route.fulfill({ json: { orgs: [] } }),
  );
  await page.route("**/api/v1/channel-bots?**", (route) =>
    route.fulfill({ json: { bots: [], total: 0 } }),
  );
});

test.afterEach(({ page }) => {
  expect(unexpectedRequests.get(page)).toEqual([]);
});

async function settled(page: import("@playwright/test").Page) {
  await expect(stopButton(page)).not.toBeVisible({ timeout: 6000 });
  await expect(composerInput(page)).toBeEnabled();
}

const CONFIRMED =
  /^Confirmed: Delete agent key 'ci-bot' \(nyxid_ag_12345678\) \(acknowledgement_id [0-9a-f-]{36}\)\. Retry it now\. Then tell the user in one or two plain sentences what you did and the result\.$/;

test("every chat runs with Full access: services and account tools need no cards", async ({
  page,
}) => {
  const modeWrites: string[] = [];
  page.on("request", (request) => {
    if (request.url().includes("/access-mode")) modeWrites.push(request.url());
  });
  await openAssistant(page, { faults: { nyxagentEnabled: true } });
  await expect(page.getByRole("combobox", { name: "Mode" })).toHaveCount(0);
  await sendMessage(page, "Use GitHub");
  await expect(page.getByText("Repository lookup succeeded.")).toBeVisible();
  await settled(page);
  await sendMessage(page, "Manage my account");
  await expect(page.getByText("Your agent keys are ready to manage.")).toBeVisible();
  await settled(page);
  await expect(page.getByRole("button", { name: "Allow", exact: true })).toHaveCount(0);
  await expect(page.locator("header").getByText("Full access", { exact: true })).toHaveCount(0);
  expect(modeWrites).toEqual([]);
});

test("a destructive action asks once, and Allow retries it with its acknowledgement", async ({
  page,
}) => {
  await openAssistant(page, { faults: { nyxagentEnabled: true } });
  await sendMessage(page, "Delete agent key ci-bot");
  const action = page.getByRole("region", { name: /Confirm: Delete agent key 'ci-bot'/ });
  await expect(action).toBeVisible();
  await settled(page);
  await action.getByRole("button", { name: "Allow", exact: true }).click();
  await expect(action).toHaveCount(0);
  // The single-use confirmation is retried with its acknowledgement ID.
  await expect(page.getByText(CONFIRMED)).toBeVisible();
  await expect(page.getByText("Deleted agent key ci-bot.")).toBeVisible();
  await expect(page.getByRole("status").filter({ hasText: /Used · Delete agent key/ })).toBeVisible();
  await settled(page);
  await page.reload();
  await expect(page.getByRole("status").filter({ hasText: /Used · Delete agent key/ })).toBeVisible();
  await expect(page.getByText("Deleted agent key ci-bot.")).toBeVisible();
  await expect(page.getByRole("button", { name: "Allow", exact: true })).toHaveCount(0);
});

test("Allow clicked while the reply is still streaming resumes once the reply finishes", async ({
  page,
}) => {
  await openAssistant(page, { faults: { nyxagentEnabled: true, progressStallMs: 6000 } });
  await sendMessage(page, "Delete agent key ci-bot");
  const card = page.getByRole("region", { name: /Confirm: Delete agent key 'ci-bot'/ });
  await expect(card).toBeVisible();
  // The card arrives before the assistant has finished its reply.
  await expect(stopButton(page)).toBeVisible();
  await card.getByRole("button", { name: "Allow", exact: true }).click();
  await expect(card).toHaveCount(0);
  await expect(page.getByText(CONFIRMED)).toHaveCount(0);
  // After the running reply settles, the confirmation is delivered as one continuation.
  await expect(page.getByRole("status").filter({ hasText: "Continuing after your decision" })).toBeVisible({ timeout: 10_000 });
  await expect(page.getByText("Deleted agent key ci-bot.")).toBeVisible({ timeout: 10_000 });
  await settled(page);
  await expect(page.getByText("Deleted agent key ci-bot.", { exact: true })).toHaveCount(1);
});

test("Deny produces a refusal on retry without opening another card", async ({ page }) => {
  await openAssistant(page, { faults: { nyxagentEnabled: true } });
  await sendMessage(page, "Delete agent key ci-bot");
  const card = page.getByRole("region", { name: /Confirm: Delete agent key 'ci-bot'/ });
  await expect(card).toBeVisible();
  await settled(page);
  await card.getByRole("button", { name: "Deny", exact: true }).click();
  await expect(composerInput(page)).toBeFocused();
  await sendMessage(page, "Continue");
  await expect(page.getByText("You denied this request. I will not proceed.")).toBeVisible();
  await expect(page.getByRole("button", { name: "Allow", exact: true })).toHaveCount(0);
});

test("NyxBot settings ask before destructive actions in Auto; Full access deletes without a card", async ({
  page,
}) => {
  await openAssistant(page, { faults: { nyxagentEnabled: true } });
  await openAccountMenu(page);
  await page.getByRole("menuitem", { name: "NyxBot settings", exact: true }).click();
  const settings = page.getByRole("dialog", { name: "NyxBot settings", exact: true });
  const mode = settings.getByRole("combobox", { name: "Access mode" });
  await expect(mode).toHaveText("Auto");
  const save = settings.getByRole("button", { name: "Save settings" });
  await expect(save).toBeDisabled();
  await mode.click();
  await page.getByRole("option", { name: "Full access", exact: true }).click();
  await expect(mode).toHaveText("Full access");
  await expect(settings.getByRole("alert")).toContainText("without asking you first");
  await save.click();
  await expect(save).toBeDisabled();

  // Channel bots: connect one (to NyxBot by default) and get a one-time owner link.
  await settings.getByRole("button", { name: "Connect NyxID Approvals" }).click();
  const link = settings.getByRole("region", { name: "Owner link" });
  await expect(link.getByRole("link", { name: "Open link" })).toHaveAttribute(
    "href",
    /^https:\/\/t\.me\/nyxid_approvals_bot\?start=nyxlink_[a-f0-9]{12}$/,
  );
  await expect(link).toContainText("press Start");
  await expect(
    settings.getByRole("list", { name: "Connected channel bots" }),
  ).toContainText("NyxID Approvals");
  await page.keyboard.press("Escape");
  await expect(page.getByRole("dialog", { name: "NyxBot settings", exact: true })).toHaveCount(0);
  await expect(page.getByRole("dialog", { includeHidden: true })).toHaveCount(0);

  await sendMessage(page, "Delete agent key ci-bot");
  await expect(page.getByText("Deleted agent key ci-bot.")).toBeVisible();
  await settled(page);
  await expect(page.getByRole("button", { name: "Allow", exact: true })).toHaveCount(0);
  await page.reload();
  await openAccountMenu(page);
  await page.getByRole("menuitem", { name: "NyxBot settings", exact: true }).click();
  await expect(
    settings.getByRole("combobox", { name: "Access mode" }),
  ).toHaveText("Full access");
});

function agentsNav(page: import("@playwright/test").Page) {
  return page.getByRole("navigation", { name: "Agents and conversations" });
}

function homeHeading(page: import("@playwright/test").Page) {
  return page.getByRole("heading", {
    level: 1,
    name: /^Good (morning|afternoon|evening), Dannick$/,
  });
}

test("the Agents list pins NyxBot, opens a specialist's thread, and forgets a memory", async ({
  page,
}) => {
  await openAssistant(page, { faults: { nyxagentEnabled: true } });
  const nav = agentsNav(page);
  const nyxbot = nav.getByRole("button", { name: "NyxBot — your personal agent" });
  await expect(nyxbot).toHaveAttribute("aria-expanded", "true");
  const researcher = nav.getByRole("button", { name: /^researcher, specialist/ });
  await expect(researcher).toBeVisible();
  // The landing is the NyxBot home.
  await expect(homeHeading(page)).toBeVisible();

  await researcher.click();
  await expect(researcher).toHaveAttribute("aria-expanded", "true");
  // Selecting an agent lands on its latest thread.
  await expect(page).toHaveURL(/c=nyxa-[a-f0-9]{32}/);
  await expect(page.getByRole("heading", { name: "researcher" })).toBeVisible();
  await expect(page.getByText("Specialist", { exact: true })).toBeVisible();
  await expect(page.getByRole("article", { name: "Message from NyxBot" })).toContainText(
    "Find the three most urgent open GitHub issues.",
  );
  await expect(page.getByText("Found 3 urgent issues: #12, #15 and #18.")).toBeVisible();
  await expect(
    nav.getByRole("group", { name: "Threads with researcher" }).getByRole("button", {
      name: "researcher",
      exact: true,
    }),
  ).toBeVisible();

  await page.getByRole("button", { name: "Agent details" }).click();
  const details = page.getByRole("dialog", { name: "Agent details" });
  const memory = details.getByRole("list", { name: "researcher memory" });
  await expect(memory).toContainText("The user wants weekly issue digests on Mondays.");
  await memory
    .getByRole("button", { name: "Forget: The user wants weekly issue digests on Mondays." })
    .click();
  await expect(memory).not.toContainText("weekly issue digests");
  await expect(memory).toContainText("Label urgent issues with the p0 tag.");
  await page.keyboard.press("Escape");
  await page.reload();
  await page.getByRole("button", { name: "Agent details" }).click();
  await expect(
    page.getByRole("dialog", { name: "Agent details" }).getByRole("list", {
      name: "researcher memory",
    }),
  ).not.toContainText("weekly issue digests");
});

test("creating an agent opens its home thread, and each agent keeps its own threads", async ({
  page,
}) => {
  await openAssistant(page, { faults: { nyxagentEnabled: true } });
  const nav = agentsNav(page);
  await nav.getByRole("button", { name: "Create new agent" }).click();
  await page.getByRole("menuitem", { name: "Personal agent" }).click();
  const dialog = page.getByRole("dialog", { name: "New agent" });
  await dialog.getByRole("textbox", { name: "Name", exact: true }).fill("writer");
  await dialog.getByRole("textbox", { name: "Role" }).fill("Drafts release notes.");
  await dialog.getByRole("checkbox", { name: /GitHub/ }).click();
  await dialog.getByRole("button", { name: "Create agent" }).click();
  await expect(page.getByRole("dialog", { includeHidden: true })).toHaveCount(0);
  await expect(page).toHaveURL(/c=nyxa-[a-f0-9]{32}/);
  await expect(page.getByRole("heading", { name: "writer" })).toBeVisible();
  const writer = nav.getByRole("button", { name: /^writer, specialist/ });
  await expect(writer).toHaveAttribute("aria-expanded", "true");

  await sendMessage(page, "Use GitHub");
  await expect(page.getByText("GitHub lookup succeeded.")).toBeVisible();
  await settled(page);
  // A second thread with the same agent.
  await nav.getByRole("button", { name: "New chat with writer" }).click();
  await expect(page.getByRole("heading", { name: "writer" })).toBeVisible();
  await sendMessage(page, "Summarize the changelog");
  await expect(page.getByText("Working on it: Summarize the changelog")).toBeVisible();
  await settled(page);
  const threads = nav.getByRole("group", { name: "Threads with writer" });
  await expect(threads.getByRole("button", { name: "Summarize the changelog", exact: true })).toBeVisible();
  await expect(threads.getByRole("button", { name: "writer", exact: true })).toBeVisible();

  // NyxBot's threads are separate.
  await nav.getByRole("button", { name: "NyxBot — your personal agent" }).click();
  await expect(page.getByRole("heading", { name: "NyxBot", exact: true })).toBeVisible();
  await expect(nav.getByRole("group", { name: "Threads with NyxBot" })).not.toContainText(
    "Summarize the changelog",
  );
});

test("a specialist's permission request is routed to NyxBot and the user can still decide it", async ({
  page,
}) => {
  await openAssistant(page, { faults: { nyxagentEnabled: true } });
  await agentsNav(page).getByRole("button", { name: /^researcher, specialist/ }).click();
  await expect(page.getByText("Found 3 urgent issues: #12, #15 and #18.")).toBeVisible();
  await sendMessage(page, "Use Slack");
  const card = page.getByRole("region", { name: "Allow this agent to use Slack?" });
  await expect(card).toContainText("Requested from NyxBot");
  await settled(page);
  await card.getByRole("button", { name: "Allow", exact: true }).click();
  await expect(card).toHaveCount(0);
  await expect(
    page
      .getByRole("status")
      .filter({ hasText: "You allowed · Use Slack" }),
  ).toBeVisible();
  // NyxID resumes the specialist itself (shown as an activity card); no
  // continuation is typed for the user.
  await expect(page.getByRole("note", { name: "NyxID event: You allowed slack" })).toBeVisible();
  await expect(page.getByText("Slack access granted. Lookup succeeded.")).toBeVisible({
    timeout: 10_000,
  });
  await expect(page.getByText(/^Approved: /)).toHaveCount(0);
});

test("NyxBot delegates to a sidebar specialist and is woken by its reply", async ({
  page,
}) => {
  await openAssistant(page, { faults: { nyxagentEnabled: true } });
  await sendMessage(page, "Ask the researcher for urgent issues");
  await expect(page.getByText("I asked the researcher to find the urgent issues.")).toBeVisible();
  const researcher = agentsNav(page).getByRole("button", { name: /^researcher, specialist/ });
  await expect(researcher).toBeVisible();
  await expect(page.getByRole("region", { name: "Team" })).toHaveCount(0);
  // The specialist's report arrives as a compact activity card whose text expands.
  const card = page.getByRole("note", { name: "NyxID event: researcher replied" });
  await expect(card).toBeVisible({ timeout: 15_000 });
  await expect(card).not.toContainText("Found 3 urgent issues");
  await card.getByRole("button", { name: "researcher replied" }).click();
  await expect(card).toContainText("Found 3 urgent issues: #12, #15 and #18.");
  await expect(page.getByText("The researcher reported back: Found 3 urgent issues: #12, #15 and #18.")).toBeVisible();
  await expect(researcher).toHaveAccessibleName(/^researcher, specialist, Idle/, { timeout: 10_000 });
  // The specialist's own thread shows NyxBot's instruction as coming from NyxBot.
  await researcher.click();
  await expect(
    page.getByRole("article", { name: "Message from NyxBot" }).last(),
  ).toContainText("From NyxBot");
});

test("channel bots can be relinked to a specialist", async ({ page }) => {
  await openAssistant(page, { faults: { nyxagentEnabled: true } });
  await openAccountMenu(page);
  await page.getByRole("menuitem", { name: "NyxBot settings", exact: true }).click();
  const settings = page.getByRole("dialog", { name: "NyxBot settings", exact: true });
  await settings.getByRole("button", { name: "Connect NyxID Approvals" }).click();
  const connected = settings.getByRole("list", { name: "Connected channel bots" });
  const agentFor = connected.getByRole("combobox", { name: "Agent for NyxID Approvals" });
  await expect(agentFor).toContainText("NyxBot");
  await agentFor.click();
  await page.getByRole("option", { name: "researcher" }).click();
  await expect(agentFor).toContainText("researcher");
  await page.keyboard.press("Escape");
  await expect(page.getByRole("dialog", { name: "NyxBot settings", exact: true })).toHaveCount(0);
  await expect(page.getByRole("dialog", { includeHidden: true })).toHaveCount(0);
  const researcher = agentsNav(page).getByRole("button", { name: /^researcher, specialist/ });
  await expect(researcher).toContainText("Telegram");
  await researcher.click();
  await page.getByRole("button", { name: "Agent details" }).click();
  const details = page.getByRole("dialog", { name: "Agent details" });
  await expect(
    details.getByRole("list", { name: "Connected channel bots" }),
  ).toContainText("NyxID Approvals");
});

test("a bot's chats: who can talk, how it answers, and posting are set per chat", async ({
  page,
}) => {
  await openAssistant(page, { faults: { nyxagentEnabled: true } });
  await openAccountMenu(page);
  await page.getByRole("menuitem", { name: "NyxBot settings", exact: true }).click();
  const settings = page.getByRole("dialog", { name: "NyxBot settings", exact: true });
  await settings.getByRole("button", { name: "Connect NyxID Approvals" }).click();
  const connected = settings.getByRole("list", { name: "Connected channel bots" });
  await connected.getByRole("button", { name: "Chats and who can talk" }).click();
  const chats = connected.getByRole("list", { name: "Chats of NyxID Approvals" });
  await expect(chats.getByRole("listitem")).toHaveCount(2);
  const replies = chats.getByRole("combobox", { name: "Replies in Team chat" });
  await expect(replies).toContainText("When mentioned");
  await replies.click();
  await page.getByRole("option", { name: "Every message" }).click();
  await expect(replies).toContainText("Every message");
  await expect(chats).toContainText("privacy mode");
  const posts = chats.getByRole("switch", { name: "Let the agent post in Team chat on its own" });
  await posts.click();
  await expect(posts).toBeChecked();
  // Private chats have no reply mode; who may talk there is set on the bot.
  await expect(chats.getByRole("combobox", { name: "Replies in You" })).toHaveCount(0);
  const access = connected.getByRole("combobox", {
    name: "Who can talk to NyxID Approvals in private chats",
  });
  await access.click();
  await page.getByRole("option", { name: "Anyone" }).click();
  await expect(access).toContainText("Anyone");
  await page.reload();
  const reopened = page.getByRole("dialog", { name: "NyxBot settings", exact: true });
  const list = reopened.getByRole("list", { name: "Connected channel bots" });
  await list.getByRole("button", { name: "Chats and who can talk" }).click();
  await expect(
    list.getByRole("combobox", { name: "Replies in Team chat" }),
  ).toContainText("Every message");
  await expect(
    list.getByRole("switch", { name: "Let the agent post in Team chat on its own" }),
  ).toBeChecked();
  await expect(
    list.getByRole("combobox", { name: "Who can talk to NyxID Approvals in private chats" }),
  ).toContainText("Anyone");
});

test("destroying a specialist makes its threads read-only; it can then be deleted", async ({
  page,
}) => {
  await openAssistant(page, { faults: { nyxagentEnabled: true } });
  const nav = agentsNav(page);
  await nav.getByRole("button", { name: /^researcher, specialist/ }).click();
  await expect(page.getByRole("heading", { name: "researcher" })).toBeVisible();
  await page.getByRole("button", { name: "Agent details" }).click();
  const details = page.getByRole("dialog", { name: "Agent details" });
  await details.getByRole("region", { name: "Danger zone" }).getByRole("button", { name: "Destroy" }).click();
  await page.getByRole("dialog", { name: "Destroy researcher?" }).getByRole("button", { name: "Destroy" }).click();
  await expect(details.getByRole("button", { name: "Delete permanently" })).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(page.getByRole("status").filter({ hasText: "this thread is read-only" })).toBeVisible();
  await expect(composerInput(page)).toBeDisabled();
  await expect(nav.getByRole("button", { name: /^researcher, specialist/ })).toHaveCount(0);
  await nav.getByRole("button", { name: "Show destroyed (1)" }).click();
  await expect(nav.getByRole("button", { name: "researcher, specialist, Destroyed" })).toBeVisible();

  await page.getByRole("button", { name: "Agent details" }).click();
  await page.getByRole("dialog", { name: "Agent details" }).getByRole("button", { name: "Delete permanently" }).click();
  await page
    .getByRole("dialog", { name: "Delete researcher permanently?" })
    .getByRole("button", { name: "Delete permanently" })
    .click();
  await expect(homeHeading(page)).toBeVisible();
  await expect(nav.getByRole("button", { name: /^researcher/ })).toHaveCount(0);
});

const DESTINATIONS = [
  { label: "Plugins", pathname: "/assistant/plugins" },
  { label: "Approvals", pathname: "/assistant/approvals" },
  { label: "Studio", pathname: "/dashboard" },
] as const;

test.describe("assistant navigation leaves an open thread and stays there", () => {
  for (const source of ["NyxBot thread", "specialist thread"] as const) {
    for (const destination of DESTINATIONS) {
      test(`${source}: ${destination.label} opens ${destination.pathname}`, async ({ page }) => {
        await openAssistant(page, { faults: { nyxagentEnabled: true } });
        if (source === "NyxBot thread") {
          await sendMessage(page, "Use GitHub");
          await expect(page.getByText("Repository lookup succeeded.")).toBeVisible();
          await settled(page);
        } else {
          await agentsNav(page).getByRole("button", { name: /^researcher, specialist/ }).click();
          await expect(page.getByRole("heading", { name: "researcher" })).toBeVisible();
        }
        await expect(page).toHaveURL(/\/assistant\?.*c=nyxa-[a-f0-9]{32}/);

        if (destination.label === "Studio") {
          await expect(page.getByRole("link", { name: "Studio", exact: true })).toHaveCount(0);
          await page.getByRole("button", { name: "Account menu" }).click();
          await page.getByRole("menuitem", { name: "Open Studio", exact: true }).click();
        } else {
          await page.getByRole("link", { name: destination.label }).click();
        }

        const pathname = () => new URL(page.url()).pathname;
        await expect.poll(pathname).toBe(destination.pathname);
        // A stale landing redirect would pull the page back to the thread.
        await page.waitForTimeout(1500);
        expect(pathname()).toBe(destination.pathname);
        expect(new URL(page.url()).searchParams.get("c")).toBeNull();
      });
    }
  }
});

test("assistant chat keys are hidden until requested and detail links back to the chat", async ({
  page,
}) => {
  await mockDashboard(page);
  const conversation = `nyxa-${"a".repeat(32)}`;
  const key = {
    id: "chat-key",
    name: "NyxID Assistant chat aaaaaaaa",
    platform: "nyxid-assistant",
    key_prefix: "nyxid_ag_fixture",
    scopes: "proxy",
    is_active: true,
    allowed_service_ids: [],
    allowed_services: [],
    allowed_node_ids: [],
    allowed_nodes: [],
    allow_all_services: false,
    allow_all_nodes: true,
    allow_auto_connected_services: true,
    bindings_count: 0,
    callback_url: null,
    created_at: "2026-09-17T00:00:00Z",
    assistant_conversation_id: conversation,
  };
  await page.route("**/api/v1/api-keys**", async (route) => {
    const path = new URL(route.request().url()).pathname;
    const body = path.endsWith("/api-keys")
      ? { keys: [key] }
      : path.endsWith("/chat-key")
        ? key
        : path.endsWith("/bindings")
          ? { bindings: [] }
          : path.endsWith("/credentials")
            ? { credentials: [] }
            : path.endsWith("/chat-key/usage")
              ? {
                  api_key_id: key.id,
                  api_key_name: key.name,
                  platform: key.platform,
                  request_count: 0,
                  success_count: 0,
                  error_count: 0,
                  error_rate: 0,
                  last_used_at: null,
                  prompt_tokens: 0,
                  completion_tokens: 0,
                  total_tokens: 0,
                  reported_cost: null,
                  top_services: [],
                  daily_buckets: [],
                }
              : { usage: [] };
    await route.fulfill({ json: body });
  });
  await page.route("**/api/v1/users/me/onboarding", (route) =>
    route.fulfill({ json: { completed: true, completed_at: "2026-09-17T00:00:00Z" } }),
  );
  await page.route("**/api/v1/keys", (route) => route.fulfill({ json: { keys: [] } }));
  await page.route("**/api/v1/catalog", (route) => route.fulfill({ json: { entries: [] } }));
  const errors: string[] = [];
  page.on("pageerror", (error) => errors.push(error.message));
  await page.goto("/keys?tab=nyxid");
  const toggle = page.getByRole("switch", { name: "Show assistant chat keys" });
  await expect(toggle).toBeVisible();
  await expect(page.getByText(key.name, { exact: true })).toHaveCount(0);
  await toggle.click();
  await page.getByText(key.name, { exact: true }).filter({ visible: true }).click();
  await expect(page.getByRole("link", { name: "Open chat" })).toHaveAttribute(
    "href",
    `/assistant?c=${conversation}`,
  );
  await expect(page.getByText("Daily Activity", { exact: true })).toBeVisible();
  expect(errors).toEqual([]);
});

test("chat width resizes the transcript and composer together, persists, and fits smaller screens", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await openAssistant(page, { faults: { nyxagentEnabled: true } });
  await sendMessage(page, "Use GitHub");
  await expect(page.getByText("Repository lookup succeeded.")).toBeVisible();
  await settled(page);
  // The message row and input follow the same column, including at its limits.
  const message = page.locator("[id^=message-]").last();
  const composer = page.locator("[data-composer-input]");
  const handle = page.getByRole("separator", { name: "Resize chat content from right" });
  const original = await composer.boundingBox();
  const grip = await handle.boundingBox();
  expect(original).not.toBeNull();
  expect(grip).not.toBeNull();
  // The tab rises from the content slab into the gap above it, overlapping the slab by 4px.
  const tab = (await handle.locator("[data-chat-width-grip]").boundingBox())!;
  expect(tab.y).toBeCloseTo(grip!.y + 4, 0);
  expect(tab.y + tab.height).toBeCloseTo((await page.getByRole("main").boundingBox())!.y + 4, 0);
  expect(grip!.x + grip!.width / 2).toBeCloseTo(original!.x + original!.width, 0);
  const guide = handle.locator("..").locator("[data-chat-width-guide]");
  await expect(guide).toHaveCSS("opacity", "0");
  // Grab the handle tab at the top of the column.
  const gripY = grip!.y + 8;
  await page.mouse.move(grip!.x + grip!.width / 2, gripY);
  await expect(guide).toHaveCSS("opacity", "1");
  await page.mouse.down();
  await page.mouse.move(grip!.x + grip!.width / 2 + 100, gripY, { steps: 8 });
  await expect(guide).toHaveCSS("opacity", "1");
  await page.mouse.up();
  await page.mouse.move(20, 20);
  await expect(guide).toHaveCSS("opacity", "0");
  const expanded = await composer.boundingBox();
  expect(expanded!.width).toBeCloseTo(original!.width + 200, 0);
  const row = await message.boundingBox();
  expect(row!.width).toBeCloseTo(expanded!.width, 0);
  expect(row!.x).toBeCloseTo(expanded!.x, 0);
  await page.reload();
  await expect(page.getByText("Repository lookup succeeded.")).toBeVisible();
  expect((await composer.boundingBox())!.width).toBeCloseTo(expanded!.width, 0);
  await handle.focus();
  await expect(guide).toHaveCSS("opacity", "1");
  await page.keyboard.press("ArrowLeft");
  expect((await composer.boundingBox())!.width).toBeCloseTo(expanded!.width - 16, 0);
  await openAccountMenu(page);
  await page.getByRole("menuitem", { name: "Settings", exact: true }).click();
  await page.getByRole("tab", { name: "Display", exact: true }).click();
  await expect(page.getByRole("slider", { name: "Chat content width" })).toHaveValue(String(Math.round(expanded!.width - 16)));
  await page.getByRole("button", { name: "Reset chat content width" }).click();
  expect((await composer.boundingBox())!.width).toBeCloseTo(original!.width, 0);
  await page.keyboard.press("Escape");
  await handle.focus();
  await page.keyboard.press("End");
  for (const width of [1024, 390, 320]) {
    await page.setViewportSize({ width, height: 844 });
    const rect = await composer.boundingBox();
    expect(rect!.x).toBeGreaterThanOrEqual(0);
    expect(rect!.x + rect!.width).toBeLessThanOrEqual(width);
    const overflow = await page.getByRole("main").evaluate((element) => element.scrollWidth - element.clientWidth);
    expect(overflow).toBeLessThanOrEqual(1);
    if (width < 768) {
      await expect(handle).toBeHidden();
    }
  }
});

test("display settings share chat width and composer height across chats and reloads", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await openAssistant(page, { faults: { nyxagentEnabled: true } });
  await sendMessage(page, "Use GitHub");
  await expect(page.getByText("Repository lookup succeeded.")).toBeVisible();
  await settled(page);

  async function openDisplay() {
    await page.getByRole("button", { name: "Account menu" }).click();
    await page.getByRole("menuitem", { name: "Settings", exact: true }).click();
    await page.getByRole("tab", { name: "Display", exact: true }).click();
  }
  const settings = page.getByRole("dialog", { name: "Account Settings" });
  await openDisplay();
  await page.getByRole("slider", { name: "Chat content width" }).press("Home");
  const heightSetting = page.getByRole("slider", { name: "Maximum composer height" });
  await heightSetting.press("Home");
  await heightSetting.press("PageUp");
  const savedHeight = Number(await heightSetting.inputValue());
  expect(savedHeight).toBeGreaterThan(64);
  expect(savedHeight).toBeLessThan(400);
  await settings.getByRole("button", { name: "Close", exact: true }).click();

  const input = composerInput(page);
  const composer = page.locator("[data-composer-input]");
  const draft = Array.from({ length: 40 }, (_, i) => `Line ${i + 1}: review the next step.`).join("\n");
  async function expectLayout(width: number, height: number) {
    await input.fill(draft);
    await expect.poll(async () => (await composer.boundingBox())!.width).toBeCloseTo(width, 0);
    await expect.poll(async () => (await input.boundingBox())!.height).toBeCloseTo(height, 0);
  }
  await expectLayout(520, savedHeight);
  await agentsNav(page).getByRole("button", { name: /^researcher, specialist/ }).click();
  await expectLayout(520, savedHeight);
  await page.getByRole("separator", { name: "Resize chat content from right" }).press("ArrowRight");
  await page.getByRole("separator", { name: "Resize composer" }).press("ArrowDown");
  await expectLayout(536, savedHeight - 24);

  await agentsNav(page).getByRole("button", { name: "New chat with researcher" }).click();
  await expectLayout(536, savedHeight - 24);
  await agentsNav(page).getByRole("button", { name: "Create new agent", exact: true }).click();
  await page.getByRole("menuitem", { name: "Crew" }).click();
  const group = page.getByRole("dialog", { name: "New crew", exact: true });
  await group.getByRole("textbox", { name: "Name", exact: true }).fill("Shared layout");
  await group.getByRole("button", { name: "Create crew", exact: true }).click();
  await expect(page.getByRole("heading", { name: "Shared layout", exact: true })).toBeVisible();
  await expectLayout(536, savedHeight - 24);
  await page.reload();
  await expectLayout(536, savedHeight - 24);

  await page.setViewportSize({ width: 390, height: 300 });
  await expect.poll(async () => (await input.boundingBox())!.height).toBeLessThan(savedHeight - 24);
  await page.setViewportSize({ width: 1440, height: 900 });
  await expectLayout(536, savedHeight - 24);
  await input.fill("Short draft");
  await expect.poll(async () => (await input.boundingBox())!.height).toBeLessThan(60);
  await openDisplay();
  await expect(page.getByRole("slider", { name: "Chat content width" })).toHaveValue("536");
  await expect(page.getByRole("slider", { name: "Maximum composer height" })).toHaveValue(String(savedHeight - 24));
  await page.getByRole("button", { name: "Reset display settings" }).click();
  await expect(page.getByRole("slider", { name: "Chat content width" })).toHaveValue("758");
  await expect(page.getByRole("slider", { name: "Maximum composer height" })).toHaveValue("92");
});

test("chat scroll fades follow hidden content at both edges and inside code blocks", async ({ page }) => {
  // Short enough that the compact sidebar still overflows.
  await page.setViewportSize({ width: 1280, height: 480 });
  await openAssistant(page, { faults: { nyxagentEnabled: true } });
  // The whole sidebar column scrolls as one area around the agents list.
  const sidebar = page.locator("[data-scroll-fade]").filter({ has: agentsNav(page) });
  await expect(sidebar).toHaveAttribute("data-scroll-edges", "bottom");
  await sidebar.evaluate((element) => { element.scrollTop = element.scrollHeight; });
  await expect(sidebar).toHaveAttribute("data-scroll-edges", "top");
  await sidebar.getByRole("button", { name: /^researcher, specialist/ }).click();
  const text = Array.from({ length: 20 }, (_, index) => `Section ${index + 1}: Check the current work and report the result.`).join("\n\n")
    + "\n\n```text\n" + "wide-output ".repeat(60) + "\n```";
  await sendMessage(page, text);
  await expect(page.getByText(/^Working on it: Section 1/)).toBeVisible();
  await settled(page);
  const transcript = page.getByRole("main").locator("[data-scroll-fade]").filter({ has: page.locator("[id^=message-]") }).first();
  await expect(transcript).toHaveAttribute("data-scroll-edges", "top");
  const replyBounds = await page.locator("[id^=message-]").last().boundingBox();
  await expect.poll(async () => (await page.locator("[data-composer-input]").boundingBox())!.x).toBeCloseTo(replyBounds!.x, 0);
  await transcript.evaluate((element) => { element.scrollTop = element.scrollHeight / 2 - element.clientHeight / 2; });
  await expect(transcript).toHaveAttribute("data-scroll-edges", "top bottom");
  expect(await transcript.evaluate((element) => getComputedStyle(element).getPropertyValue("--scroll-fade-top").trim())).toBe("20px");
  await transcript.evaluate((element) => { element.scrollTop = 0; });
  await expect(transcript).toHaveAttribute("data-scroll-edges", "bottom");
  const code = page.getByRole("main").locator("pre[data-scroll-fade]");
  await expect(code).toHaveAttribute("data-scroll-edges", "right");
  await code.evaluate((element) => { element.scrollLeft = element.scrollWidth; });
  await expect(code).toHaveAttribute("data-scroll-edges", "left");
  await page.setViewportSize({ width: 1280, height: 2000 });
  await expect(sidebar).toHaveAttribute("data-scroll-edges", "");
});

test("conversation index previews and navigates messages outside the rendered window", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 800 });
  await openAssistant(page, { faults: { nyxagentEnabled: true } });
  await sendMessage(page, "Use GitHub");
  await settled(page);
  const rail = page.getByRole("navigation", { name: "Conversation index" });
  await expect(rail).toHaveAttribute("data-scroll-edges", "");
  const regularPitch = (await rail.locator("[data-chat-index-target]").first().boundingBox())!.height;
  await page.evaluate(() => {
    const fixture = JSON.parse(sessionStorage.getItem("nyxagent-http-fixture-v4")!);
    const id = new URLSearchParams(location.search).get("c");
    const row = fixture.rows.find(([key]: [string]) => key === id)[1];
    row.history.messages = Array.from({ length: 160 }, (_, index) => ({
      id: `index-message-${index}`, seq: index + 1, turn_id: `index-turn-${Math.floor(index / 2)}`,
      role: index % 2 ? "assistant" : "user", text: `Review item ${index + 1}: ${index % 2 ? "The findings are ready to check." : "Please check the next item."}`,
      status: "completed", error_code: null, created_at: new Date(1_790_000_000_000 + index * 1000).toISOString(), activities: [], attachments: [],
    }));
    row.history.conversation.message_count = row.history.messages.length;
    row.history.acknowledgements = [];
    sessionStorage.setItem("nyxagent-http-fixture-v4", JSON.stringify(fixture));
  });
  await page.reload();
  const transcript = page.locator("[data-chat-transcript]");
  await expect(rail).toBeVisible();
  await expect(rail.getByRole("button", { name: /Earlier messages|Later messages/ })).toHaveCount(0);
  await expect(rail).toHaveAttribute("data-scroll-edges", "top");
  expect((await rail.locator("[data-chat-index-target]").first().boundingBox())!.height).toBeLessThan(regularPitch);
  expect(await rail.evaluate((element) => getComputedStyle(element).maskImage)).not.toBe("none");
  await expect(rail.locator('[aria-current="location"]')).toHaveAttribute("data-message-id", "index-message-159");
  expect(await page.locator("[data-chat-message-index]").count()).toBeLessThan(30);
  async function expectRailFitsAboveComposer() {
    const bounds = (await rail.boundingBox())!;
    const composer = (await page.locator("[data-composer-surface]").boundingBox())!;
    const transcriptBounds = (await transcript.boundingBox())!;
    expect(bounds.y).toBeCloseTo(transcriptBounds.y, 0);
    expect(bounds.y + bounds.height).toBeCloseTo(composer.y, 0);
    const marks = rail.locator("[data-chat-index-target]");
    const pitch = (await marks.first().boundingBox())!.height;
    const occupied = await marks.count() * pitch;
    expect(occupied).toBeLessThanOrEqual(bounds.height);
    expect(bounds.height - occupied).toBeLessThan(pitch);
    return bounds.height;
  }
  await expectRailFitsAboveComposer();
  // A passing pointer previews nothing; a pressed tick previews its message.
  const lastTick = rail.locator('[data-message-id="index-message-159"]');
  await lastTick.hover();
  await expect(page.getByRole("tooltip")).toHaveCount(0);
  const lastBox = (await lastTick.boundingBox())!;
  await page.mouse.move(lastBox.x + lastBox.width / 2, lastBox.y + lastBox.height / 2);
  await page.mouse.down();
  await expect(page.getByRole("tooltip")).toContainText("Review item 160");
  await page.mouse.up();
  await expect(page.getByRole("tooltip")).toHaveCount(0);

  const initialScroll = await transcript.evaluate((element) => element.scrollTop);
  const firstIndex = Number(await rail.locator("[data-chat-index-target]").first().getAttribute("data-chat-index-target"));
  await page.mouse.wheel(0, -160);
  await expect(rail).toHaveAttribute("data-scroll-edges", "top bottom");
  await expect.poll(async () => Number(await rail.locator("[data-chat-index-target]").first().getAttribute("data-chat-index-target"))).toBeLessThan(firstIndex);
  expect(await transcript.evaluate((element) => element.scrollTop)).toBe(initialScroll);
  await expect(rail.locator('[data-chat-index-target][tabindex="0"]')).toHaveCount(1);
  const target = rail.locator("[data-chat-index-target]").first();
  const id = await target.getAttribute("data-message-id");
  await expect(page.locator(`#message-${id}`)).toHaveCount(0);
  await target.click();
  await expect(page.locator(`#message-${id}`)).toBeInViewport();
  await expect(rail.locator('[aria-current="location"]')).toHaveAttribute("data-message-id", id!);

  await rail.locator('[aria-current="location"]').focus();
  await page.keyboard.press("Home");
  await expect(rail).toHaveAttribute("data-scroll-edges", "bottom");
  await expect(page.locator("#message-index-message-0")).toBeInViewport();
  await expect(rail.locator('[data-message-id="index-message-0"]')).toBeFocused();
  const userTick = await rail.locator('[data-message-id="index-message-0"] span').boundingBox();
  const agentTick = await rail.locator('[data-message-id="index-message-1"] span').boundingBox();
  // Keyboard focus magnifies the current tick.
  expect(agentTick!.width).toBeGreaterThan(userTick!.width);
  await page.keyboard.press("ArrowDown");
  await expect(page.locator("#message-index-message-1")).toBeInViewport();
  await expect(rail.locator('[data-message-id="index-message-1"]')).toBeFocused();

  const dragFrom = (await rail.locator('[data-message-id="index-message-2"]').boundingBox())!;
  const dragTo = (await rail.locator('[data-message-id="index-message-8"]').boundingBox())!;
  await page.mouse.move(dragFrom.x + 12, dragFrom.y + dragFrom.height / 2);
  await page.mouse.down();
  await page.mouse.move(dragTo.x + 12, dragTo.y + dragTo.height / 2, { steps: 6 });
  await page.mouse.up();
  await expect(page.locator("#message-index-message-8")).toBeInViewport();
  await rail.locator('[aria-current="location"]').focus();
  await page.keyboard.press("End");
  await expect(rail).toHaveAttribute("data-scroll-edges", "top");
  await expect(page.locator("#message-index-message-159")).toBeInViewport();
  await expect.poll(() => transcript.evaluate((element) => element.scrollHeight - element.clientHeight - element.scrollTop)).toBeLessThanOrEqual(1);
  const initialRailHeight = await expectRailFitsAboveComposer();
  await composerInput(page).fill("Review the next item.\n".repeat(40));
  await page.getByRole("separator", { name: "Resize composer" }).press("End");
  await expect.poll(async () => (await rail.boundingBox())!.height).toBeLessThan(initialRailHeight - 250);
  await expect(async () => { await expectRailFitsAboveComposer(); }).toPass();
  await composerInput(page).fill("");
  await page.setViewportSize({ width: 1440, height: 1200 });
  await expect(rail).toHaveAttribute("data-scroll-edges", "");
  await expect(rail.locator("[data-chat-index-target]")).toHaveCount(160);
  const fittedPitch = (await rail.locator("[data-chat-index-target]").first().boundingBox())!.height;
  expect(fittedPitch).toBeGreaterThan(5);
  expect(fittedPitch).toBeLessThan(regularPitch);
  await expectRailFitsAboveComposer();
  await page.setViewportSize({ width: 1440, height: 1600 });
  await expect(rail).toHaveAttribute("data-scroll-edges", "");
  await expect(rail.locator("[data-chat-index-target]")).toHaveCount(160);
  await expect.poll(async () => (await rail.locator("[data-chat-index-target]").first().boundingBox())!.height).toBe(regularPitch);
  await page.setViewportSize({ width: 390, height: 700 });
  await expect(rail).toBeHidden();
  expect(await page.getByRole("main").evaluate((element) => element.scrollWidth - element.clientWidth)).toBeLessThanOrEqual(1);
});

test("jump to latest returns a reader who scrolled up to the newest message", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 800 });
  await openAssistant(page, { faults: { nyxagentEnabled: true } });
  await agentsNav(page).getByRole("button", { name: /^researcher, specialist/ }).click();
  const text = Array.from({ length: 30 }, (_, index) => `Section ${index + 1}: Check the current work and report the result.`).join("\n\n");
  await sendMessage(page, text);
  await settled(page);
  const transcript = page.locator("[data-chat-transcript]");
  const jump = page.getByRole("button", { name: "Jump to latest message" });
  await expect(jump).toHaveCount(0);

  await transcript.evaluate((element) => { element.scrollTop = 0; });
  await expect(jump).toBeVisible();
  const button = (await jump.boundingBox())!;
  const composer = (await page.locator("[data-composer-input]").boundingBox())!;
  expect(button.x + button.width / 2).toBeCloseTo(composer.x + composer.width / 2, 0);
  expect(button.y + button.height).toBeLessThan(composer.y);

  // The header collapses on the way down; the jump must still land at the end.
  await jump.click();
  await expect.poll(() => transcript.evaluate((element) => element.scrollHeight - element.clientHeight - element.scrollTop)).toBeLessThanOrEqual(1);
  await expect(jump).toHaveCount(0);
  await expect(page.getByRole("group", { name: "Conversation controls" })).toHaveAttribute("data-compact", "true");
});

test("conversation controls float after scrolling without moving the reading position", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 800 });
  await openAssistant(page, { faults: { nyxagentEnabled: true } });
  await agentsNav(page).getByRole("button", { name: /^researcher, specialist/ }).click();
  const text = Array.from({ length: 20 }, (_, index) => `Section ${index + 1}: Check the current work and report the result.`).join("\n\n");
  await sendMessage(page, text);
  await expect(page.getByText(/^Working on it: Section 1/)).toBeVisible();
  await settled(page);
  const controls = page.getByRole("group", { name: "Conversation controls" });
  const transcript = page.locator("[data-chat-transcript]");
  await expect(controls).toHaveAttribute("data-compact", "true");
  await expect.poll(() => transcript.evaluate((element) => element.scrollHeight - element.clientHeight - element.scrollTop)).toBeLessThanOrEqual(1);

  await transcript.evaluate((element) => { element.scrollTop = 0; });
  await expect(controls).toHaveAttribute("data-compact", "false");
  await expect.poll(() => transcript.evaluate((element) => element.scrollTop)).toBe(0);
  const expandedHeight = await transcript.evaluate((element) => element.clientHeight);
  await transcript.evaluate((element) => { element.scrollTop = 120; });
  await expect(controls).toHaveAttribute("data-compact", "false");
  const message = page.locator("[id^=message-]").first();
  const before = (await message.boundingBox())!.y;
  const viewport = (await transcript.boundingBox())!;
  await page.mouse.move(viewport.x + viewport.width / 2, viewport.y + 200);
  await page.mouse.wheel(0, 100);
  await expect(controls).toHaveAttribute("data-compact", "true");
  await expect.poll(async () => (await message.boundingBox())!.y).toBeCloseTo(before - 100, 0);
  expect(await transcript.evaluate((element) => element.clientHeight)).toBeGreaterThan(expandedHeight);

  // Small movements around the collapse point must not toggle the row again.
  await page.mouse.wheel(0, -30);
  await expect.poll(() => transcript.evaluate((element) => element.scrollTop)).toBeLessThan(160);
  await expect(controls).toHaveAttribute("data-compact", "true");
  for (const name of ["Working context", "Agent details"]) {
    const trigger = controls.getByRole("button", { name });
    await trigger.click();
    await expect(page.getByRole("dialog", { name, exact: true })).toBeVisible();
    await page.keyboard.press("Escape");
    await expect(trigger).toBeFocused();
    await expect(controls).toHaveAttribute("data-compact", "true");
  }

  // Returning near the start restores the row and keeps the text anchored.
  const restoredY = await transcript.evaluate((element) => {
    element.scrollTop = 20;
    return element.querySelector("[id^=message-]")!.getBoundingClientRect().y;
  });
  await expect(controls).toHaveAttribute("data-compact", "false");
  await expect.poll(async () => (await message.boundingBox())!.y).toBeCloseTo(restoredY, 0);

  for (const width of [390, 320]) {
    await page.setViewportSize({ width, height: 700 });
    await transcript.evaluate((element) => { element.scrollTop = element.scrollHeight; });
    await expect(controls).toHaveAttribute("data-compact", "true");
    const box = (await controls.boundingBox())!;
    expect(box.x).toBeGreaterThanOrEqual(0);
    expect(box.x + box.width).toBeLessThanOrEqual(width);
    await controls.getByRole("button", { name: "Working context" }).click();
    await expect(page.getByRole("dialog", { name: "Working context", exact: true })).toBeVisible();
    await page.keyboard.press("Escape");
    expect(await page.getByRole("main").evaluate((element) => element.scrollWidth - element.clientWidth)).toBeLessThanOrEqual(1);
    await transcript.evaluate((element) => { element.scrollTop = 0; });
    await expect(controls).toHaveAttribute("data-compact", "false");
    await expect.poll(() => transcript.evaluate((element) => element.scrollTop)).toBe(0);
  }

  await page.setViewportSize({ width: 1440, height: 800 });
  await sendMessage(page, "Continue checking");
  await settled(page);
  await expect.poll(() => transcript.evaluate((element) => element.scrollHeight - element.clientHeight - element.scrollTop)).toBeLessThanOrEqual(1);
  await expect(controls).toHaveAttribute("data-compact", "true");
  await agentsNav(page).getByRole("button", { name: "New chat with researcher" }).click();
  await expect(controls).toHaveAttribute("data-compact", "false");
  await expect.poll(() => transcript.evaluate((element) => element.scrollTop)).toBe(0);
});
