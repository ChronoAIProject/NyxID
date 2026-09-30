import { test, expect, type Page } from "@playwright/test";
import {
  openAssistant,
  sendMessage,
  stopButton,
  composerInput,
  conversationRow,
  SEEDED,
} from "./helpers";

const RESET_NOTICE =
  "Conversation context was reset; the assistant was given a recap of this chat.";
const GREETING = /^Good (morning|afternoon|evening), Dannick$/;

function homeHeading(page: Page) {
  return page.getByRole("heading", { level: 1, name: GREETING });
}

function nav(page: Page) {
  return page.getByRole("navigation");
}

test("the home composer starts a NyxBot thread: durable reload, connect link, rename and delete", async ({
  page,
}) => {
  await openAssistant(page, { faults: { nyxagentEnabled: true } });
  await expect(homeHeading(page)).toBeVisible();
  // Model routing is server-side: no profile selector.
  await expect(page.getByRole("combobox", { name: "Profile" })).toHaveCount(0);
  await expect(composerInput(page)).toHaveAttribute("placeholder", "Message NyxBot");
  await sendMessage(page, "List connected services");
  await expect(page).toHaveURL(/c=nyxa-[a-f0-9]{32}/);
  await expect(page.getByRole("heading", { name: "NyxBot", exact: true })).toBeVisible();
  const connect = page.getByRole("link", { name: "Connect GitHub" });
  await expect(connect).toBeVisible();
  await page.reload();
  await expect(connect).toHaveAttribute("rel", "noopener noreferrer");
  await conversationRow(page, "List connected services").hover();
  await page.getByRole("button", { name: "Options for List connected services" }).click();
  await page.getByRole("menuitem", { name: "Rename" }).click();
  await expect(page.getByRole("menu", { includeHidden: true })).toHaveCount(0);
  await page.getByRole("dialog").getByRole("textbox").fill("Services overview");
  await page.getByRole("button", { name: "Save", exact: true }).click();
  await expect(page.getByRole("dialog", { includeHidden: true })).toHaveCount(0);
  await expect(conversationRow(page, "Services overview")).toBeVisible();
  await conversationRow(page, "Services overview").hover();
  await page.getByRole("button", { name: "Options for Services overview" }).click();
  await page.getByRole("menuitem", { name: "Delete", exact: true }).click();
  await expect(page.getByRole("menu", { includeHidden: true })).toHaveCount(0);
  await page.getByRole("dialog").getByRole("button", { name: "Delete", exact: true }).click();
  await expect(page.getByRole("dialog", { includeHidden: true })).toHaveCount(0);
  await expect(conversationRow(page, "Services overview")).toHaveCount(0);
  // Deleting the open NyxBot thread returns to the home.
  await expect(homeHeading(page)).toBeVisible();
});

test("Stop and reload retain one inline reset note between the failed reply and next user message", async ({
  page,
}) => {
  await openAssistant(page, { faults: { nyxagentEnabled: true, progressStallMs: 4000 } });
  await sendMessage(page, "Inspect my services");
  await expect(page).toHaveURL(/c=nyxa-/);
  await page.reload();
  await expect(stopButton(page)).toBeVisible();
  await stopButton(page).click();
  await expect(stopButton(page)).not.toBeVisible({ timeout: 6000 });
  const partial = page.getByText("Your connected services", { exact: true });
  const note = page.getByRole("note", { name: "Conversation context reset" });
  await expect(partial).toBeVisible();
  await expect(note).toHaveText(RESET_NOTICE);
  await expect(note).toHaveCount(1);
  await expect(composerInput(page)).toBeEnabled();
  await sendMessage(page, "Continue the inspection");
  await expect(note).toHaveCount(1);
  await expect(page.getByRole("link", { name: "Connect GitHub" })).toBeVisible({ timeout: 8000 });
  await page.reload();
  await expect(note).toHaveCount(1);
  // DOM order proves this is a transcript note, rather than a permanent top banner.
  await expect
    .poll(async () =>
      page.evaluate(() => {
        const note = document.querySelector(
          '[role="note"][aria-label="Conversation context reset"]',
        );
        const leaves = [...document.querySelectorAll("p, div")].filter(
          (element) => element.childElementCount === 0,
        );
        const failed = leaves.find((element) => element.textContent === "Your connected services");
        const next = leaves.find((element) => element.textContent === "Continue the inspection");
        return Boolean(
          note &&
          failed &&
          next &&
          failed.compareDocumentPosition(note) & Node.DOCUMENT_POSITION_FOLLOWING &&
          note.compareDocumentPosition(next) & Node.DOCUMENT_POSITION_FOLLOWING,
        );
      }),
    )
    .toBe(true);
});

test("legacy chats are no longer listed with NyxAgent enabled", async ({ page }) => {
  await openAssistant(page, { faults: { nyxagentEnabled: true } });
  await expect(homeHeading(page)).toBeVisible();
  // No "Chats" section, no generic New chat, none of the earlier engines' chats.
  await expect(nav(page).getByText("Chats", { exact: true })).toHaveCount(0);
  await expect(page.getByRole("button", { name: "New chat", exact: true })).toHaveCount(0);
  for (const seeded of Object.values(SEEDED)) {
    await expect(conversationRow(page, seeded.title)).toHaveCount(0);
  }
  await expect(nav(page).getByRole("button", { name: "NyxBot — your personal agent" })).toBeVisible();

  // The workspace pages use the same NyxAgent sidebar.
  await page.getByRole("link", { name: "Plugins" }).click();
  await expect.poll(() => new URL(page.url()).pathname).toBe("/assistant/plugins");
  await expect(nav(page).getByRole("button", { name: /^researcher, specialist/ })).toBeVisible();
  await expect(nav(page).getByText("Chats", { exact: true })).toHaveCount(0);
  for (const seeded of Object.values(SEEDED)) {
    await expect(conversationRow(page, seeded.title)).toHaveCount(0);
  }
});

test("images a tool returns show under the reply during the turn and after reload", async ({
  page,
}) => {
  await openAssistant(page, { faults: { nyxagentEnabled: true, progressStallMs: 4000 } });
  await sendMessage(page, "Show the lobby camera");
  const image = page.getByRole("img", { name: "Image from lobby-camera__snapshot" });
  // The polled live turn carries the attachment before the reply settles.
  await expect(image).toBeVisible({ timeout: 6000 });
  await expect(page.getByText("Here is the latest lobby snapshot.")).toBeVisible({
    timeout: 8000,
  });
  await expect(image).toHaveJSProperty("naturalWidth", 1);
  await page.reload();
  await expect(page.getByText("Here is the latest lobby snapshot.")).toBeVisible();
  await expect(image).toBeVisible();
  await expect(image).toHaveJSProperty("naturalWidth", 1);
});

test("the NyxBot home shows what happened, what NyxBot remembers, the roster and groups", async ({
  page,
}) => {
  await openAssistant(page, { faults: { nyxagentEnabled: true } });
  await expect(homeHeading(page)).toBeVisible();
  await expect(page.getByRole("button", { name: "Home", exact: true })).toHaveAttribute(
    "aria-current",
    "page",
  );
  const away = page.getByRole("region", { name: "While you were away" });
  await expect(away).toContainText("researcher");
  await expect(away).toContainText("Found 3 urgent issues: #12, #15 and #18.");
  const memory = page.getByRole("region", { name: "What NyxBot remembers" });
  await expect(memory).toContainText("Nothing yet");
  const agents = page.getByRole("region", { name: "Your agents" });
  await expect(agents.getByRole("listitem")).toHaveCount(2);
  await expect(agents).toContainText("Tracks open GitHub issues");
  const groups = page.getByRole("region", { name: "Groups" });
  await expect(groups).toContainText("Put several agents in one chat");

  // NyxBot remembers across threads; the home previews it and links to Manage.
  await sendMessage(page, "Remember I prefer short answers");
  await expect(page.getByText("Noted. I will remember that.")).toBeVisible();
  await page.getByRole("button", { name: "Home", exact: true }).click();
  await expect(homeHeading(page)).toBeVisible();
  await expect(memory).toContainText("I prefer short answers");
  await memory.getByRole("button", { name: "Manage" }).click();
  await expect(
    page.getByRole("dialog", { name: "Agent details" }).getByRole("list", { name: "NyxBot memory" }),
  ).toContainText("I prefer short answers");
  await page.keyboard.press("Escape");

  // A group made from the home shows up as a card.
  await groups.getByRole("button", { name: "New group" }).click();
  const dialog = page.getByRole("dialog", { name: "New group" });
  await dialog.getByRole("textbox", { name: "Name", exact: true }).fill("Standup");
  await dialog.getByRole("button", { name: "Create group" }).click();
  await expect(page).toHaveURL(/g=nyxg-[a-f0-9]{32}/);
  await page.getByRole("button", { name: "Home", exact: true }).click();
  await expect(groups.getByRole("button", { name: "Open group Standup" })).toBeVisible();

  // Roster "Chat" opens the agent's latest thread.
  await agents.getByRole("button", { name: "Chat with researcher" }).click();
  await expect(page).toHaveURL(/c=nyxa-[a-f0-9]{32}/);
  await expect(page.getByRole("heading", { name: "researcher" })).toBeVisible();
  await expect(composerInput(page)).toHaveAttribute("placeholder", "Message researcher");

  // Deep links still work: ?draft opens a fresh NyxBot thread, not the home.
  await page.goto("/assistant?mock=1&draft=true");
  await expect(page.getByRole("heading", { name: "NyxBot", exact: true })).toBeVisible();
  await expect(homeHeading(page)).toHaveCount(0);
});

async function createGroup(page: Page, name: string, extra: string[]) {
  await nav(page).getByRole("button", { name: "New group" }).click();
  const dialog = page.getByRole("dialog", { name: "New group" });
  // NyxBot is preselected.
  await expect(dialog.getByRole("checkbox", { name: /NyxBot/ })).toBeChecked();
  await dialog.getByRole("textbox", { name: "Name", exact: true }).fill(name);
  for (const agent of extra) {
    await dialog.getByRole("checkbox", { name: new RegExp(agent) }).click();
  }
  await dialog.getByRole("button", { name: "Create group" }).click();
  await expect(page).toHaveURL(/g=nyxg-[a-f0-9]{32}/);
  await expect(page.getByRole("heading", { name, exact: true })).toBeVisible();
}

test("a group routes @mentions, shows who is working, and can be renamed, pruned and deleted", async ({
  page,
}) => {
  await openAssistant(page, { faults: { nyxagentEnabled: true } });
  await createGroup(page, "Launch crew", ["researcher"]);
  await expect(page.getByRole("note", { name: "Group notice" })).toHaveText(
    "Group created with NyxBot, researcher.",
  );
  const input = composerInput(page);
  await expect(input).toHaveAttribute("placeholder", "Message the group — @mention an agent");

  // Typing @ lists members; Tab inserts the mention.
  await input.fill("@res");
  const mentions = page.getByRole("listbox", { name: "Mention an agent" });
  await expect(mentions.getByRole("option")).toHaveText([/researcher/]);
  await input.press("Tab");
  await expect(input).toHaveValue("@researcher ");
  await expect(mentions).toHaveCount(0);
  await input.pressSequentially("find something");
  await input.press("Enter");

  // Only the mentioned member answers, and the group shows it working meanwhile.
  await expect(page.getByRole("status", { name: "Agents working" })).toContainText(
    "researcher is working…",
  );
  await expect(page.getByText("1 working", { exact: true })).toBeVisible();
  await expect(page.getByRole("article", { name: "Message from researcher" })).toContainText(
    "Working on it: find something",
  );
  await expect(page.getByRole("status", { name: "Agents working" })).toHaveCount(0);
  await expect(page.getByRole("article", { name: "Message from NyxBot" })).toHaveCount(0);

  // No mention: the lead (NyxBot) answers, and hands the work on with @researcher.
  await input.fill("Find the urgent issues");
  await input.press("Enter");
  await expect(page.getByRole("article", { name: "Message from NyxBot" })).toContainText(
    "@researcher can you find the urgent issues?",
  );
  await expect(
    page.getByRole("article", { name: "Message from researcher" }).last(),
  ).toContainText("Found 3 urgent issues: #12, #15 and #18.", { timeout: 10_000 });

  // Member avatars open the agent's details.
  await page
    .getByRole("group", { name: "Members" })
    .getByRole("button", { name: "researcher — agent details" })
    .click();
  await expect(
    page.getByRole("dialog", { name: "Agent details" }).getByRole("list", {
      name: "researcher memory",
    }),
  ).toBeVisible();
  await page.keyboard.press("Escape");

  // Rename and remove a member.
  await page.getByRole("button", { name: "Group settings" }).click();
  const settings = page.getByRole("dialog", { name: "Group settings" });
  await settings.getByRole("textbox", { name: "Name", exact: true }).fill("Release crew");
  await settings.getByRole("checkbox", { name: /researcher/ }).click();
  await settings.getByRole("button", { name: "Save", exact: true }).click();
  await expect(page.getByRole("dialog", { includeHidden: true })).toHaveCount(0);
  await expect(page.getByRole("heading", { name: "Release crew", exact: true })).toBeVisible();
  await expect(
    page.getByRole("note", { name: "Group notice" }).filter({ hasText: "researcher left." }),
  ).toBeVisible();
  await expect(
    page.getByRole("group", { name: "Members" }).getByRole("button"),
  ).toHaveCount(1);
  await expect(nav(page).getByRole("button", { name: /^Release crew, group with NyxBot$/ })).toBeVisible();
  await input.fill("@");
  await expect(mentions.getByRole("option")).toHaveText([/NyxBot/]);
  await input.fill("");

  // Delete (confirmed) returns to the home.
  await page.getByRole("button", { name: "Group settings" }).click();
  await page.getByRole("dialog", { name: "Group settings" }).getByRole("button", { name: "Delete", exact: true }).click();
  await page
    .getByRole("dialog", { name: "Delete Release crew?" })
    .getByRole("button", { name: "Delete group" })
    .click();
  await expect(homeHeading(page)).toBeVisible();
  await expect(nav(page).getByRole("button", { name: /^Release crew/ })).toHaveCount(0);
});

test("a destroyed member stays visible, cannot be mentioned, and leaves on the next save", async ({
  page,
}) => {
  await openAssistant(page, { faults: { nyxagentEnabled: true } });
  await createGroup(page, "Research desk", ["researcher"]);
  const members = page.getByRole("group", { name: "Members" });
  await members.getByRole("button", { name: "researcher — agent details" }).click();
  const details = page.getByRole("dialog", { name: "Agent details" });
  await details.getByRole("region", { name: "Danger zone" }).getByRole("button", { name: "Destroy" }).click();
  await page.getByRole("dialog", { name: "Destroy researcher?" }).getByRole("button", { name: "Destroy" }).click();
  await expect(details.getByRole("button", { name: "Delete permanently" })).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(
    members.getByRole("button", { name: "researcher (destroyed) — agent details" }),
  ).toBeVisible();

  // Only live members are offered, and a message for the destroyed one goes to the lead.
  const input = composerInput(page);
  await input.fill("@");
  await expect(
    page.getByRole("listbox", { name: "Mention an agent" }).getByRole("option"),
  ).toHaveText([/NyxBot/]);
  await input.fill("@researcher are you there?");
  await input.press("Enter");
  await expect(page.getByRole("article", { name: "Message from NyxBot" })).toContainText(
    "Noted: are you there?",
  );

  // The server refuses destroyed agents in a member list, so settings drop it on save.
  await page.getByRole("button", { name: "Group settings" }).click();
  const settings = page.getByRole("dialog", { name: "Group settings" });
  const researcher = settings.getByRole("checkbox", { name: /researcher/ });
  await expect(researcher).not.toBeChecked();
  await expect(researcher).toBeDisabled();
  await settings.getByRole("button", { name: "Save", exact: true }).click();
  await expect(page.getByRole("dialog", { includeHidden: true })).toHaveCount(0);
  await expect(members.getByRole("button")).toHaveCount(1);
  await expect(
    page.getByRole("note", { name: "Group notice" }).filter({ hasText: "researcher left." }),
  ).toBeVisible();
});

test("a thread that came from a chat app sits in its bot's section and shows its platform", async ({
  page,
}) => {
  await openAssistant(page, { faults: { nyxagentEnabled: true } });
  // Chat-app threads are grouped per bot, collapsed until opened.
  const section = page
    .getByRole("navigation")
    .getByRole("button", { name: "Telegram bot on Telegram, 1 chat" });
  await expect(section).toHaveAttribute("aria-expanded", "false");
  await expect(conversationRow(page, "Morning briefing")).toHaveCount(0);
  await section.click();
  await expect(section).toHaveAttribute("aria-expanded", "true");
  const row = conversationRow(page, "Morning briefing");
  await row.click();
  await expect(page).toHaveURL(/c=nyxa-[a-f0-9]{32}/);
  const main = page.getByRole("main");
  await expect(main.getByText("via Telegram")).toHaveCount(2);
  await expect(main.getByText("What is on my calendar today?")).toBeVisible();
  await expect(composerInput(page)).toHaveAttribute("placeholder", "Message NyxBot");
});

test("mobile drawer: a group created from the drawer's New group opens", async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await openAssistant(page, { faults: { nyxagentEnabled: true } });
  await page.getByRole("button", { name: "Open chats" }).click();
  await nav(page).last().getByRole("button", { name: "New group" }).click();
  const dialog = page.getByRole("dialog", { name: "New group" });
  await dialog.getByRole("textbox", { name: "Name", exact: true }).fill("Pocket crew");
  // Clicks inside the dialog must not close the drawer that owns it.
  await dialog.getByRole("button", { name: "Create group" }).click();
  await expect(page).toHaveURL(/g=nyxg-[a-f0-9]{32}/);
  await expect(page.getByRole("heading", { name: "Pocket crew", exact: true })).toBeVisible();
});

test("agents get a display name and persona, shown with their @handle everywhere", async ({
  page,
}) => {
  await openAssistant(page, { faults: { nyxagentEnabled: true } });
  await nav(page).getByRole("button", { name: "New agent" }).click();
  const dialog = page.getByRole("dialog", { name: "New agent" });
  await dialog.getByRole("textbox", { name: "Name", exact: true }).fill("writer");
  await dialog.getByRole("textbox", { name: /Display name/ }).fill("Luna");
  await dialog.getByRole("textbox", { name: "Role" }).fill("Drafts release notes.");
  await dialog.getByRole("textbox", { name: /Persona/ }).fill("Warm, concise.");
  await dialog.getByRole("button", { name: "Create agent" }).click();
  await expect(page).toHaveURL(/c=nyxa-[a-f0-9]{32}/);
  await expect(page.getByRole("heading", { name: "Luna", exact: true })).toBeVisible();
  await expect(page.getByRole("main").getByText("@writer", { exact: true })).toBeVisible();
  await expect(composerInput(page)).toHaveAttribute("placeholder", "Message Luna");
  await expect(nav(page).getByRole("button", { name: "Luna (@writer), specialist, Idle" })).toBeVisible();

  // The drawer edits both; they persist.
  await page.getByRole("button", { name: "Agent details" }).click();
  const profile = page.getByRole("dialog", { name: "Agent details" }).getByRole("form", { name: "Profile" });
  await expect(profile.getByRole("textbox", { name: "Persona" })).toHaveValue("Warm, concise.");
  await profile.getByRole("textbox", { name: "Persona" }).fill("Playful.");
  await profile.getByRole("button", { name: "Save" }).click();
  await expect(profile.getByRole("button", { name: "Save" })).toBeDisabled();
  await page.keyboard.press("Escape");
  await page.reload();
  await page.getByRole("button", { name: "Agent details" }).click();
  await expect(
    page.getByRole("dialog", { name: "Agent details" }).getByRole("textbox", { name: "Persona" }),
  ).toHaveValue("Playful.");
  await page.keyboard.press("Escape");

  // NyxBot can have one too (its @handle stays NyxBot).
  await page.getByRole("button", { name: "Home", exact: true }).click();
  await page.getByRole("region", { name: "What NyxBot remembers" }).getByRole("button", { name: "Manage" }).click();
  const nyxProfile = page.getByRole("dialog", { name: "Agent details" }).getByRole("form", { name: "Profile" });
  await nyxProfile.getByRole("textbox", { name: "Display name" }).fill("Nyx");
  await nyxProfile.getByRole("button", { name: "Save" }).click();
  await expect(nyxProfile.getByRole("button", { name: "Save" })).toBeDisabled();
  await page.keyboard.press("Escape");
  await expect(nav(page).getByRole("button", { name: "Nyx (@NyxBot) — your personal agent" })).toBeVisible();
  const roster = page.getByRole("region", { name: "Your agents" });
  await expect(roster.getByRole("button", { name: "Chat with Nyx" })).toBeVisible();
  await expect(roster.getByRole("button", { name: "Chat with Luna" })).toBeVisible();

  // Groups show display names and still mention by @handle.
  await createGroup(page, "Docs", ["Luna"]);
  const input = composerInput(page);
  await input.fill("@lu");
  const option = page.getByRole("listbox", { name: "Mention an agent" }).getByRole("option");
  await expect(option).toHaveText([/Luna@writer/]);
  await input.press("Enter");
  await expect(input).toHaveValue("@writer ");
  await input.pressSequentially("hello");
  await input.press("Enter");
  await expect(page.getByRole("article", { name: "Message from Luna" })).toContainText(
    "Working on it: hello",
  );
});

test("a thread shows what it is waiting for and resumes by itself when it happens", async ({
  page,
}) => {
  await openAssistant(page, { faults: { nyxagentEnabled: true } });
  await sendMessage(page, "Set up a Telegram bot");
  await expect(page.getByText(/to create your bot/)).toBeVisible();
  const waiting = page.getByRole("status", { name: "Waiting" });
  await expect(waiting).toContainText("Waiting for your Telegram bot to be created");
  await expect(waiting).toContainText("NyxBot continues here by itself");
  await page.getByRole("link", { name: "Telegram bot setup" }).click();
  const setup = page.getByRole("dialog");
  await setup.getByLabel("Bot token", { exact: true }).fill("demo-bot-token");
  await setup.getByRole("button", { name: "Add Bot", exact: true }).click();
  await setup.getByRole("button", { name: "Done", exact: true }).click();
  // NyxID notices the bot and resumes the thread; the user never replied.
  await expect(page.getByText(/Your Telegram bot @helper_bot is linked/)).toBeVisible({
    timeout: 15_000,
  });
  await expect(waiting).toHaveCount(0);
});

test("the transcript never shows through around or below the composer", async ({ page }) => {
  await openAssistant(page, { faults: { nyxagentEnabled: true } });
  for (const text of ["List connected services", "Use GitHub", "Manage my account"]) {
    await sendMessage(page, text);
    await expect(page.getByRole("button", { name: "Stop" })).toHaveCount(0, { timeout: 15_000 });
  }
  // Scroll the transcript up so its text sits behind the composer.
  await page.mouse.move(400, 300);
  await page.mouse.wheel(0, -200);
  const band = page.locator("[data-composer-band]");
  const background = await band.evaluate((element) => getComputedStyle(element).backgroundColor);
  expect(background).not.toBe("rgba(0, 0, 0, 0)");
  expect(background).not.toBe("transparent");
  await expect(band.locator("[data-composer-fade]")).toHaveCount(1);
});

test("with NyxID's live stream a waiting thread resumes as soon as it happens, not on a poll", async ({
  page,
}) => {
  await openAssistant(page, { faults: { nyxagentEnabled: true, nyxagentLive: true } });
  await sendMessage(page, "Set up a Telegram bot");
  const waiting = page.getByRole("status", { name: "Waiting" });
  await expect(waiting).toContainText("Waiting for your Telegram bot to be created");
  await page.getByRole("link", { name: "Telegram bot setup" }).click();
  const setup = page.getByRole("dialog");
  await setup.getByLabel("Bot token", { exact: true }).fill("demo-bot-token");
  await setup.getByRole("button", { name: "Add Bot", exact: true }).click();
  await setup.getByRole("button", { name: "Done", exact: true }).click();
  // Only the user's submission creates the bot. The live stream resumes the
  // thread before the ten-second waiting poll would observe that mutation.
  await expect(page.getByText(/Your Telegram bot @helper_bot is linked/)).toBeVisible({
    timeout: 7000,
  });
  await expect(waiting).toHaveCount(0);
});
