import { test, expect } from "@playwright/test";
import { openAssistant, sendMessage, composerInput, stopButton } from "./helpers";
import { mockDashboard } from "./managed-onboarding-fixtures";

async function settled(page: import("@playwright/test").Page) {
  await expect(stopButton(page)).not.toBeVisible({ timeout: 6000 });
  await expect(composerInput(page)).toBeEnabled();
}

const CONFIRMED =
  /^Confirmed: Delete agent key 'ci-bot' \(nyxid_ag_12345678\) \(acknowledgement_id [0-9a-f-]{36}\)\. Retry it now\.$/;

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
  await expect(page.getByRole("status").filter({ hasText: /Used · Confirm:/ })).toBeVisible();
  await settled(page);
  await page.reload();
  await expect(page.getByRole("status").filter({ hasText: /Used · Confirm:/ })).toBeVisible();
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
  await expect(page.getByText(CONFIRMED)).toBeVisible({ timeout: 10_000 });
  await expect(page.getByText("Deleted agent key ci-bot.")).toBeVisible({ timeout: 10_000 });
  await settled(page);
  await expect(page.getByText(CONFIRMED)).toHaveCount(1);
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

test("NyxBot settings confirm destructive actions by default; turning that off deletes without a card", async ({
  page,
}) => {
  await openAssistant(page, { faults: { nyxagentEnabled: true } });
  await page.getByRole("button", { name: "NyxBot settings" }).click();
  const dialog = page.getByRole("dialog", { name: "NyxBot settings" });
  const toggle = dialog.getByRole("switch", { name: "Confirm destructive actions" });
  await expect(toggle).toBeChecked();
  const save = dialog.getByRole("button", { name: "Save settings" });
  await expect(save).toBeDisabled();
  await toggle.click();
  await expect(toggle).not.toBeChecked();
  await expect(dialog.getByRole("alert")).toContainText("without asking you first");
  await save.click();
  await expect(save).toBeDisabled();

  // Channel bots: connect one (to NyxBot by default) and get a one-time owner link.
  await dialog.getByRole("button", { name: "Connect NyxID Approvals" }).click();
  const link = dialog.getByRole("region", { name: "Owner link" });
  await expect(link.getByRole("link", { name: "Open link" })).toHaveAttribute(
    "href",
    /^https:\/\/t\.me\/nyxid_approvals_bot\?start=nyxlink_[a-f0-9]{12}$/,
  );
  await expect(link).toContainText("press Start");
  await expect(
    dialog.getByRole("list", { name: "Connected channel bots" }),
  ).toContainText("NyxID Approvals");
  await page.keyboard.press("Escape");
  await expect(page.getByRole("dialog", { includeHidden: true })).toHaveCount(0);

  await sendMessage(page, "Delete agent key ci-bot");
  await expect(page.getByText("Deleted agent key ci-bot.")).toBeVisible();
  await settled(page);
  await expect(page.getByRole("button", { name: "Allow", exact: true })).toHaveCount(0);
  await page.reload();
  await page.getByRole("button", { name: "NyxBot settings" }).click();
  await expect(
    page.getByRole("dialog").getByRole("switch", { name: "Confirm destructive actions" }),
  ).not.toBeChecked();
});

function agentsNav(page: import("@playwright/test").Page) {
  return page.getByRole("navigation");
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
  await nav.getByRole("button", { name: "New agent" }).click();
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
      .filter({ hasText: "Allowed by you · Allow this agent to use Slack?" }),
  ).toBeVisible();
  // NyxID resumes the specialist itself (shown as an activity card); no
  // continuation is typed for the user.
  await expect(page.getByRole("note", { name: "NyxID event: You allowed slack" })).toBeVisible();
  await expect(page.getByText("Slack access granted. Lookup succeeded.")).toBeVisible({
    timeout: 10_000,
  });
  await expect(page.getByText(/^Approved: /)).toHaveCount(0);
});

test("NyxBot delegates to a specialist, shows it working in the Team strip, and is woken by its reply", async ({
  page,
}) => {
  await openAssistant(page, { faults: { nyxagentEnabled: true } });
  await sendMessage(page, "Ask the researcher for urgent issues");
  await expect(page.getByText("I asked the researcher to find the urgent issues.")).toBeVisible();
  const team = page.getByRole("region", { name: "Team" });
  await expect(team).toContainText("researcher");
  // The specialist's report arrives as a compact activity card whose text expands.
  const card = page.getByRole("note", { name: "NyxID event: researcher replied" });
  await expect(card).toBeVisible({ timeout: 15_000 });
  await expect(card).not.toContainText("Found 3 urgent issues");
  await card.getByRole("button", { name: "researcher replied" }).click();
  await expect(card).toContainText("Found 3 urgent issues: #12, #15 and #18.");
  await expect(page.getByText("The researcher reported back: Found 3 urgent issues: #12, #15 and #18.")).toBeVisible();
  await expect(team).toContainText("Idle", { timeout: 10_000 });
  // The specialist's own thread shows NyxBot's instruction as coming from NyxBot.
  await agentsNav(page).getByRole("button", { name: /^researcher, specialist/ }).click();
  await expect(
    page.getByRole("article", { name: "Message from NyxBot" }).last(),
  ).toContainText("From NyxBot");
});

test("channel bots can be relinked to a specialist", async ({ page }) => {
  await openAssistant(page, { faults: { nyxagentEnabled: true } });
  await page.getByRole("button", { name: "NyxBot settings" }).click();
  const dialog = page.getByRole("dialog", { name: "NyxBot settings" });
  await dialog.getByRole("button", { name: "Connect NyxID Approvals" }).click();
  const connected = dialog.getByRole("list", { name: "Connected channel bots" });
  const agentFor = connected.getByRole("combobox", { name: "Agent for NyxID Approvals" });
  await expect(agentFor).toContainText("NyxBot");
  await agentFor.click();
  await page.getByRole("option", { name: "researcher" }).click();
  await expect(agentFor).toContainText("researcher");
  await page.keyboard.press("Escape");
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
  await page.getByRole("button", { name: "NyxBot settings" }).click();
  const dialog = page.getByRole("dialog", { name: "NyxBot settings" });
  await dialog.getByRole("button", { name: "Connect NyxID Approvals" }).click();
  const connected = dialog.getByRole("list", { name: "Connected channel bots" });
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
  await page.getByRole("button", { name: "NyxBot settings" }).click();
  const reopened = page.getByRole("dialog", { name: "NyxBot settings" });
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

test.describe("sidebar navigation leaves an open thread and stays there", () => {
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

        await page.getByRole("link", { name: destination.label }).click();

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
