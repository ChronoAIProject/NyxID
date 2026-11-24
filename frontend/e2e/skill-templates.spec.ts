import { expect, test, type Page } from "@playwright/test";
import { composerInput, sendButton } from "./helpers";

// OAuth fixtures perform several full navigations through the dev module graph.
test.describe.configure({ timeout: 90_000 });
const research = {
  slug: "research-paper-writing-rigor",
  skillId: "a22344ff-4cf3-4ea7-a22e-2f0f98792c10",
  version: "2.0",
  sha256: "0820b8532ad9601d32749207469ebaaf15d66c49d5a077051f69a47ca845ad0e",
};
const claims = {
  slug: "research-claim-fidelity-reviewer",
  skillId: "be585a18-7c28-45e3-a8f4-e098904e3304",
  version: "0.1",
  sha256: "585d20211e65ae72f52c7d02f5b3517c121a927b313f4a08a059b99a403dcf27",
};
const owner = "d4f5a6b7-c8d9-4e0f-a1b2-c3d4e5f60718";
const agentId = "11111111-1111-4111-8111-111111111111";
const threadId = "nyxa-01010101010101010101010101010101";
const now = "2026-01-01T00:00:00Z";
const limits = {
  skip_destructive_confirmation: false,
  max_live_subagents: 8,
  max_concurrent_subagent_turns: 3,
  max_live_subagents_limit: 16,
  max_concurrent_subagent_turns_limit: 8,
};
const other = {
  source: "ornn",
  skill_id: "22222222-2222-4222-8222-222222222222",
  name: "another-skill",
  version: "1.0",
  sha256: "b".repeat(64),
  dependencies: [],
  disabled: false,
};
const pin = {
  ...other,
  skill_id: research.skillId,
  name: research.slug,
  version: research.version,
  sha256: research.sha256,
};

async function fixture(page: Page, options: {
  signedIn?: boolean;
  dismissedOnboarding?: boolean;
  callbackPath?: string;
  confirmName?: boolean;
  pauseAtProvider?: boolean;
  unavailable?: boolean;
  conflict?: boolean;
  badPreview?: boolean;
  previewError?: number;
  turnError?: number | "admitted";
  skillCount?: number;
  missingPersonalAgent?: boolean;
  existing?: Partial<typeof pin>;
  profileGate?: Promise<void>;
} = {}) {
  let signedIn = options.signedIn ?? true;
  let displayName = "Owner";
  let revision = 2;
  let conflict = options.conflict;
  let badPreview = options.badPreview;
  let previewError = options.previewError;
  let turnError = options.turnError;
  let skills = [other, ...(options.existing ? [{ ...pin, ...options.existing }] : [])];
  if (options.skillCount) {
    skills = Array.from({ length: options.skillCount }, (_, index) => ({
      ...other,
      skill_id: `22222222-2222-4222-8222-${String(index).padStart(12, "0")}`,
    }));
  }
  const writes: { expected_revision: number; skills: typeof skills }[] = [];
  const turns: { agent_id?: string; conversation_id?: string; text: string }[] = [];
  const agentReads: string[] = [];
  const previewRequests: [string | null, string | null][] = [];
  const socialRequests: URL[] = [];
  const agent = {
    id: agentId,
    name: "NyxBot",
    kind: "nyxbot",
    status: "idle",
    owner_id: owner,
    owner_kind: "person",
    created_at: now,
    last_active_at: now,
  };
  const conversation = {
    id: threadId,
    title: "Template chat",
    model: "test-model",
    created_at: now,
    last_message_at: now,
    message_count: 2,
    pending_acknowledgements: 0,
    active_turn: null,
    context_reset_at: null,
    agent,
    role: "orchestrator",
  };
  const publishedReply = 'data: {"event":"turn.completed","cursor":1,"turn_id":"turn-template","status":"completed","error":null}\n\n';
  // Exercise the browser HTTP boundary instead of the default assistant fixture.
  await page.addInitScript(() => Object.assign(window, {
    __nyxidAssistantHttpFixtureWorld: { handler: () => undefined },
  }));
  await page.route("**/api/v1/**", async (route) => {
    const request = route.request();
    const url = new URL(request.url());
    const path = url.pathname.replace(/^\/api\/v1/, "");
    const ok = (json: unknown) => route.fulfill({ json });
    if (path === "/users/me") await options.profileGate;
    if (path === "/users/me" && signedIn) {
      if (request.method() === "PUT") displayName = request.postDataJSON().display_name;
      return ok({
        id: owner,
        email: "owner@example.com",
        display_name: displayName,
        social_provider: options.confirmName ? "google" : null,
        capabilities: { enabled_features: options.unavailable ? [] : ["assistant:nyxagent-engine"] },
      });
    }
    if (path === "/users/me" || path === "/auth/refresh") {
      return route.fulfill({ status: 401, json: { error: "unauthorized", error_code: 1001, message: "Sign in" } });
    }
    if (path === "/public/config") {
      return ok({ social_providers: ["google"], invite_code_required: true, email_auth_enabled: false });
    }
    if (path === "/auth/social/google") {
      socialRequests.push(url);
      if (options.pauseAtProvider) {
        return route.fulfill({ contentType: "text/html", body: "<h1>Provider sign-in</h1>" });
      }
      signedIn = true;
      const returnTo = url.searchParams.get("return_to") ?? "/";
      const destination = options.confirmName
        ? `/register?${new URLSearchParams({ confirm_name: "provider", return_to: returnTo })}`
        : options.callbackPath ?? returnTo;
      return route.fulfill({ status: 302, headers: { location: destination } });
    }
    if (path === "/assistant/nyxagent/skills/catalog") {
      const skill = url.searchParams.get("skill");
      const version = url.searchParams.get("version");
      previewRequests.push([skill, version]);
      if (previewError) {
        const status = previewError;
        previewError = undefined;
        return route.fulfill({ status, json: status === 400 ? {
          error: "validation_error",
          error_code: 1003,
          message: "Package mismatch",
          details: { reason: "pinned_version_mismatch" },
        } : {} });
      }
      const template = [research, claims].find((entry) => entry.skillId === skill && entry.version === version);
      if (!template) return route.fulfill({ status: 404, json: {} });
      const sha256 = badPreview ? other.sha256 : template.sha256;
      badPreview = false;
      return ok({
        reference: { ...pin, skill_id: template.skillId, name: template.slug, version, sha256 },
        description: "Published skill",
        size_bytes: 400,
      });
    }
    if (path === "/assistant/nyxagent/agents") {
      agentReads.push(path);
      const roster = options.missingPersonalAgent ? [{ ...agent, kind: "specialist" }] : [agent];
      return ok({ agents: roster, limits });
    }
    if (path === "/assistant/nyxagent/settings") return ok(limits);
    if (path === `/assistant/nyxagent/agents/${agentId}/skills`) {
      if (request.method() === "PUT") {
        const selection = request.postDataJSON();
        writes.push(selection);
        if (conflict) {
          conflict = false;
          revision += 1;
          skills = [...skills, { ...other, skill_id: "33333333-3333-4333-8333-333333333333" }];
          return route.fulfill({ status: 409, json: { error: "conflict", error_code: 1005, message: "Skills changed" } });
        }
        expect(selection.expected_revision).toBe(revision);
        skills = selection.skills;
        revision += 1;
      }
      return ok({ revision, skills, metadata: {} });
    }
    if (path === "/assistant/nyxagent/turns") {
      turns.push(request.postDataJSON());
      const failure = turnError;
      turnError = undefined;
      if (typeof failure === "number") return route.fulfill({ status: failure, json: {} });
      if (failure === "admitted") {
        return route.fulfill({ contentType: "text/event-stream", body: publishedReply });
      }
      return route.fulfill({
        headers: { "Content-Type": "text/event-stream", "X-NyxID-Conversation-Id": threadId },
        body: publishedReply,
      });
    }
    if (path === `/assistant/nyxagent/conversations/${threadId}`) {
      return ok({
        conversation,
        before_seq: null,
        messages: [
          { id: "message-starter", seq: 1, turn_id: "turn-template", role: "user", text: turns[0]?.text ?? "", created_at: now, status: "completed", error_code: null },
          { id: "message-reply", seq: 2, turn_id: "turn-template", role: "assistant", text: "Please share your draft.", created_at: now, status: "completed", error_code: null },
        ],
      });
    }
    if (path === "/assistant/nyxagent/conversations") {
      return ok({ conversations: turns.length ? [conversation] : [], next_cursor: null });
    }
    if (path === "/assistant/nyxagent/groups") return ok({ groups: [] });
    if (path === "/assistant/nyxagent/onboarding") {
      if (!options.dismissedOnboarding) return ok({ show_intro: true });
      return ok({
        show_intro: false, suggestions_hidden: true, intro_version: 1,
        profile: { outcome: "skipped", topics: [], work_style: null, intro_version: 1, updated_at: now },
        topics: [], work_styles: [], recommendations: [],
      });
    }
    return route.fulfill({ status: 404, json: {} });
  });
  return { agentReads, previewRequests, socialRequests, writes, turns, restoreSession: () => { signedIn = true; } };
}

async function openWelcome(page: Page, title = "Strengthen a research paper") {
  await page.getByRole("link", { name: `Try this · ${title}` }).click();
  const dialog = page.getByRole("dialog", { name: "Awesome, let’s get you started." });
  await expect(dialog).toBeVisible();
  await expect(dialog.getByRole("region", { name: "Selected template" }).getByRole("heading")).toHaveText(title);
}

test("Learn lets a returning owner browse by keyboard and send an edited starter without losing their draft", async ({ page }) => {
  const state = await fixture(page, { dismissedOnboarding: true });
  const existingDraft = "Follow up on yesterday’s notes.";
  await page.goto("/assistant");
  await composerInput(page).fill(existingDraft);
  await page.getByRole("link", { name: "Learn", exact: true }).press("Enter");
  await expect(page).toHaveURL(/\/assistant\/learn$/);
  await page.setViewportSize({ width: 375, height: 812 });
  await page.getByRole("link", { name: "Research", exact: true }).press("Enter");
  const category = page.getByRole("region", { name: "Research", exact: true });
  const opener = category.getByRole("button", { name: "Try this · Audit research claims" });
  await opener.press("Enter");
  const dialog = page.getByRole("dialog", { name: "Audit research claims" });
  await expect(dialog).toContainText("Once added, the skill stays");
  await expect(dialog.getByRole("button", { name: "Cancel" })).toBeFocused();
  await page.keyboard.press("Shift+Tab");
  const close = dialog.getByRole("button", { name: "Close", exact: true });
  await expect(close).toBeFocused();
  expect(await close.evaluate((button) => getComputedStyle(button).boxShadow)).not.toBe("none");
  await page.keyboard.press("Escape");
  await expect(dialog).toHaveCount(0);
  await expect(opener).toBeFocused();
  expect(await opener.evaluate((button) => {
    const bounds = button.getBoundingClientRect();
    return button.matches(":focus-visible") && getComputedStyle(button).boxShadow !== "none" &&
      bounds.left >= 0 && bounds.right <= window.innerWidth;
  })).toBe(true);
  expect(state.previewRequests).toEqual([]);
  expect(state.writes).toEqual([]);
  expect(state.turns).toEqual([]);

  await opener.press("Enter");
  await dialog.getByRole("button", { name: "Open NyxBot" }).click();
  await expect(composerInput(page)).toContainText(claims.slug, { timeout: 15_000 });
  expect(state.writes).toHaveLength(1);
  expect(state.writes[0]?.skills.at(-1)).toMatchObject({ skill_id: claims.skillId, version: claims.version, sha256: claims.sha256 });
  expect(state.turns).toEqual([]);
  const edited = "Check the wording of my abstract against the attached study.";
  await composerInput(page).fill(edited);
  await sendButton(page).click();
  await expect(page).toHaveURL(new RegExp(`/assistant\\?c=${threadId}$`));
  await page.reload();
  await expect(page.getByText("Please share your draft.")).toBeVisible({ timeout: 15_000 });
  expect(state.turns).toEqual([{ agent_id: agentId, text: edited }]);
  await page.goto("/assistant");
  await expect(composerInput(page)).toHaveText(existingDraft);
  expect(state.writes).toHaveLength(1);
});

for (const scenario of [
  { name: "catalog denial", options: { previewError: 403 }, message: "Check your Ornn access", action: "Choose another template" },
  { name: "the 16-skill limit", options: { skillCount: 16 }, message: "has 16 skills", action: "Manage NyxBot skills" },
]) {
  test(`Learn explains ${scenario.name} and links to recovery without opening a starter`, async ({ page }) => {
    const state = await fixture(page, { dismissedOnboarding: true, ...scenario.options });
    await page.goto("/assistant/learn");
    await page.getByRole("region", { name: "Research", exact: true })
      .getByRole("button", { name: "Try this · Audit research claims" }).click();
    await page.getByRole("dialog").getByRole("button", { name: "Open NyxBot" }).click();
    await expect(page.getByRole("alert")).toContainText(scenario.message);
    const link = page.getByRole("link", { name: scenario.action });
    const destination = new URL((await link.getAttribute("href"))!, page.url());
    if (scenario.options.skillCount) {
      expect(destination.pathname).toBe("/assistant");
      expect(destination.searchParams.get("agent")).toBe(agentId);
      expect(destination.searchParams.get("details")).toBe("skills");
    } else {
      expect(destination.pathname).toBe("/assistant/learn");
    }
    await expect(composerInput(page)).toHaveCount(0);
    expect(state.writes).toEqual([]);
    expect(state.turns).toEqual([]);
  });
}

test("Learn shows the template starter with the NyxBot engine unavailable", async ({ page }) => {
  const state = await fixture(page, { dismissedOnboarding: true, unavailable: true });
  await page.goto("/assistant/learn");
  await page.getByRole("region", { name: "Research", exact: true })
    .getByRole("button", { name: "Try this · Audit research claims" }).click();
  const dialog = page.getByRole("dialog", { name: "Audit research claims" });
  await expect(dialog.getByRole("status")).toContainText("unavailable for this account");
  await expect(dialog).toContainText(claims.slug);
  await expect(dialog.getByRole("button", { name: "Open NyxBot" })).toHaveCount(0);
  expect(state.previewRequests).toEqual([]);
  expect(state.writes).toEqual([]);
  expect(state.turns).toEqual([]);
});

test("the gallery stays mounted and the modal supports focus, escape, Back, Forward and deep links", async ({ page }) => {
  const state = await fixture(page, { signedIn: false });
  await page.goto("/testing");
  const opener = page.getByRole("link", { name: "Try this · Strengthen a research paper" });
  await opener.focus();
  await opener.press("Enter");
  const dialog = page.getByRole("dialog", { name: "Awesome, let’s get you started." });
  await expect(dialog).toBeVisible();
  await expect(dialog.getByRole("heading", { name: "Awesome, let’s get you started." })).toBeFocused();
  await expect(page.locator('[aria-hidden="true"] main')).toBeVisible();
  await page.keyboard.press("Tab");
  await expect(dialog.getByRole("button", { name: "Create an account" })).toBeFocused();
  await page.keyboard.press("Shift+Tab");
  await expect(dialog.getByRole("button", { name: "Close" })).toBeFocused();
  await page.keyboard.press("Tab");
  await expect(dialog.getByRole("button", { name: "Create an account" })).toBeFocused();
  const url = page.url();
  await page.keyboard.press("Escape");
  await expect(page).toHaveURL(/\/testing$/);
  await expect(dialog).toHaveCount(0);
  await expect(opener).toBeFocused();
  await page.goForward();
  await expect(dialog).toBeVisible();
  await page.goBack();
  await expect(opener).toBeFocused();

  const direct = await page.context().newPage();
  await fixture(direct, { signedIn: false });
  await direct.goto(url);
  const directDialog = direct.getByRole("dialog", { name: "Awesome, let’s get you started." });
  await expect(directDialog).toBeVisible();
  await direct.reload();
  await expect(directDialog).toBeVisible();
  await directDialog.getByRole("button", { name: "Close" }).click();
  await expect(direct).toHaveURL(new RegExp(`/testing\\?skill=${research.slug}$`));
  await expect(direct.getByRole("link", { name: "Try this · Strengthen a research paper" })).toBeFocused();
  expect(state.writes).toEqual([]);
  expect(state.turns).toEqual([]);
});

test("new social accounts confirm their display name before the selected skill attaches", async ({ page }) => {
  const state = await fixture(page, { signedIn: false, confirmName: true });
  await page.goto("/testing?code=TEST-1234");
  await openWelcome(page);
  await page.getByRole("button", { name: "Create an account" }).click();
  await page.getByRole("button", { name: "Continue with Google" }).click();
  await expect(page.getByRole("heading", { name: "Choose your NyxID display name" })).toBeVisible({ timeout: 15_000 });
  expect(state.writes).toEqual([]);
  await page.getByRole("textbox", { name: "Display name" }).fill("New owner");
  await page.getByRole("button", { name: "Save and continue" }).click();
  await expect(composerInput(page)).toContainText(research.slug, { timeout: 15_000 });
  expect(state.writes[0]?.skills.at(-1)?.skill_id).toBe(research.skillId);
  expect(state.turns).toEqual([]);
});

test("leaving a template setup for ordinary login opens the regular NyxBot home", async ({ page }) => {
  const state = await fixture(page, { signedIn: false, callbackPath: "/" });
  await page.goto("/testing");
  await openWelcome(page);
  await page.getByRole("button", { name: "Close" }).click();
  await expect(page).toHaveURL(/\/testing$/);
  expect((await page.context().cookies()).filter(({ name }) => name.startsWith("nyxid_skill_setup_"))).toHaveLength(0);
  await openWelcome(page);
  await page.getByRole("button", { name: "Log in", exact: true }).click();
  await expect(page).toHaveURL(/\/login\?return_to=/);
  await page.goBack();
  await expect(page.getByRole("dialog")).toBeVisible();
  await page.getByRole("button", { name: "Close" }).click();
  await page.goto("/login");
  await page.getByRole("button", { name: "Continue with Google" }).click();
  await expect(page).toHaveURL(/\/assistant$/);
  await expect(composerInput(page)).toBeVisible({ timeout: 15_000 });
  expect(state.previewRequests).toEqual([]);
  expect(state.writes).toEqual([]);
  expect(state.turns).toEqual([]);
  await composerInput(page).fill("A regular NyxBot chat");
  await sendButton(page).click();
  await expect.poll(() => state.turns[0]?.text).toBe("A regular NyxBot chat");
});

test("returning from an abandoned provider clears the fallback before a later session is restored", async ({ page }) => {
  const state = await fixture(page, { signedIn: false, pauseAtProvider: true });
  await page.goto("/testing");
  await openWelcome(page);
  await page.getByRole("button", { name: "Log in", exact: true }).click();
  await page.getByRole("button", { name: "Continue with Google" }).click();
  await expect(page.getByRole("heading", { name: "Provider sign-in" })).toBeVisible();
  await page.goBack();
  await expect(page.getByRole("button", { name: "Continue with Google" })).toBeVisible();
  // The account is signed in from another tab while this tab is idle on Login.
  state.restoreSession();
  await page.goto("/");
  await expect(page).toHaveURL(/\/assistant$/);
  await expect(composerInput(page)).toBeVisible({ timeout: 15_000 });
  expect(state.previewRequests).toEqual([]);
  expect(state.writes).toEqual([]);
});

for (const [label, path, callbackPath] of [
  ["Create an account", "/register", undefined],
  ["Log in", "/login", "/"],
  ["Log in", "/login", "/assistant"],
] as const) {
  test(`a guest can ${label.toLowerCase()} and open the private chat via ${callbackPath ?? "the explicit return"}`, async ({ page }) => {
    const state = await fixture(page, { signedIn: false, callbackPath });
    await page.goto(`/testing?skill=${research.slug}&code=TEST-1234`);
    await expect(page.getByRole("link", { name: /Try this ·/ })).toHaveCount(7);
    await openWelcome(page);
    await expect(page.getByRole("list", { name: "Setup progress" }).getByRole("listitem")).toHaveCount(3);
    await expect(page.locator('li[aria-current="step"]')).toContainText("Log in or create an account");
    expect(state.agentReads).toEqual([]);
    expect(state.writes).toEqual([]);
    await page.getByRole("button", { name: label, exact: true }).click();
    await expect(page).toHaveURL(new RegExp(path + "\\?"));
    const auth = new URL(page.url());
    const returnTo = new URL(auth.searchParams.get("return_to")!);
    expect(returnTo.origin).toBe(auth.origin);
    expect(returnTo.pathname).toBe("/assistant");
    expect(returnTo.searchParams.get("skill")).toBe(research.slug);
    expect(returnTo.searchParams.get("skillSetup")).toMatch(/^[0-9a-f]{32}$/);
    expect(auth.searchParams.get("code")).toBe("TEST-1234");
    const cookie = (await page.context().cookies()).find(({ name }) => name === `nyxid_skill_setup_${returnTo.searchParams.get("skillSetup")}`);
    expect(cookie).toMatchObject({ domain: auth.hostname, sameSite: "Lax", value: research.slug });
    const remainingSeconds = cookie!.expires - Date.now() / 1000;
    expect(remainingSeconds).toBeGreaterThan(1740);
    expect(remainingSeconds).toBeLessThanOrEqual(1800);
    await page.getByRole("button", { name: "Continue with Google" }).click();
    // A full SSO navigation cold-loads the dev server's assistant modules.
    await expect(composerInput(page)).toContainText(research.slug, { timeout: 15_000 });
    if (path === "/register") expect(state.socialRequests[0]?.searchParams.get("invite_code")).toBe("TEST-1234");
    expect(state.writes).toEqual([{ expected_revision: 2, skills: [other, pin] }]);
    expect(state.turns).toEqual([]);
    expect((await page.context().cookies()).find(({ name }) => name === cookie?.name)).toBeUndefined();

    const editedStarter = `Use ${research.slug} to review my draft.`;
    await composerInput(page).fill(editedStarter);
    await page.reload();
    await expect(composerInput(page)).toHaveText(editedStarter, { timeout: 15_000 });
    expect(state.writes).toHaveLength(1);
    expect(state.turns).toEqual([]);

    await sendButton(page).click();
    await expect(page).toHaveURL(new RegExp(`/assistant\\?c=${threadId}$`));
    await expect(page.getByText("Please share your draft.")).toBeVisible({ timeout: 15_000 });
    expect(state.turns).toEqual([{
      agent_id: agentId,
      text: editedStarter,
    }]);
    await page.reload();
    await expect(page.getByText("Please share your draft.")).toBeVisible({ timeout: 15_000 });
    expect(state.writes).toHaveLength(1);
    expect(state.turns).toHaveLength(1);
  });
}

test("two tabs preserve separate selections and neither can claim the other tab’s setup", async ({ page, context }) => {
  const otherTab = await context.newPage();
  const first = await fixture(page, { signedIn: false, callbackPath: "/" });
  const second = await fixture(otherTab, { signedIn: false, callbackPath: "/assistant" });
  await Promise.all([page.goto("/testing"), otherTab.goto("/testing")]);
  await openWelcome(page);
  await openWelcome(otherTab, "Audit research claims");
  await page.getByRole("button", { name: "Log in", exact: true }).click();
  await otherTab.getByRole("button", { name: "Log in", exact: true }).click();
  const destination = (tab: Page) => new URL(new URL(tab.url()).searchParams.get("return_to")!);
  expect(destination(page).searchParams.get("skillSetup")).not.toBe(destination(otherTab).searchParams.get("skillSetup"));
  expect((await context.cookies()).filter(({ name }) => name.startsWith("nyxid_skill_setup_"))).toHaveLength(2);

  await page.getByRole("button", { name: "Continue with Google" }).click();
  await expect(composerInput(page)).toContainText(research.slug, { timeout: 15_000 });
  await otherTab.getByRole("button", { name: "Continue with Google" }).click();
  await expect(composerInput(otherTab)).toContainText(claims.slug, { timeout: 15_000 });
  expect(first.writes[0]?.skills.at(-1)?.skill_id).toBe(research.skillId);
  expect(second.writes[0]?.skills.at(-1)?.skill_id).toBe(claims.skillId);
  expect(first.turns).toEqual([]);
  expect(second.turns).toEqual([]);
  expect((await context.cookies()).filter(({ name }) => name.startsWith("nyxid_skill_setup_"))).toHaveLength(0);
});

test("a callback with an explicit unrelated chat preserves that destination", async ({ page }) => {
  const state = await fixture(page, { signedIn: false, callbackPath: `/assistant?c=${threadId}` });
  await page.goto("/testing");
  await openWelcome(page);
  await page.getByRole("button", { name: "Log in", exact: true }).click();
  await page.getByRole("button", { name: "Continue with Google" }).click();
  await expect(page).toHaveURL(new RegExp(`/assistant\\?c=${threadId}$`));
  await expect(page.getByText("Please share your draft.")).toBeVisible();
  expect(state.previewRequests).toEqual([]);
  expect(state.writes).toEqual([]);
  expect((await page.context().cookies()).filter(({ name }) => name.startsWith("nyxid_skill_setup_"))).toHaveLength(0);
});

test("an existing session selects a template on the welcome page and can retry a revision conflict", async ({ page }) => {
  const state = await fixture(page, { conflict: true });
  await page.goto(`/testing?skill=${claims.slug}`);
  await expect(page.getByRole("heading", { level: 2 }).first()).toHaveText("Audit research claims");
  await openWelcome(page, "Audit research claims");
  await expect(page.locator('li[aria-current="step"]')).toContainText("Open NyxBot");
  await page.getByRole("button", { name: "Open NyxBot" }).click();
  await expect(page.getByRole("alert")).toContainText("skills changed");
  expect(state.turns).toEqual([]);
  await page.getByRole("button", { name: "Try again" }).click();
  await expect(composerInput(page)).toContainText(claims.slug);
  expect(state.writes.map(({ expected_revision }) => expected_revision)).toEqual([2, 3]);
  expect(state.writes[1]?.skills).toHaveLength(3);
  expect(state.writes[1]?.skills[2]).toMatchObject({ skill_id: claims.skillId, version: claims.version, sha256: claims.sha256 });
  expect(state.turns).toEqual([]);
  await page.goBack();
  await expect(page.getByRole("heading", { name: "Awesome, let’s get you started." })).toBeVisible();
  expect(state.turns).toEqual([]);
});

test("a choice made while the session is restoring uses the restored account", async ({ page }) => {
  const profile = Promise.withResolvers<void>();
  const state = await fixture(page, { profileGate: profile.promise });
  try {
    await page.goto("/testing");
    await openWelcome(page);
    await page.getByRole("button", { name: "Log in", exact: true }).click();
    await expect(page).toHaveURL(/\/login\?/);
  } finally {
    profile.resolve();
  }
  await expect(composerInput(page)).toContainText(research.slug);
  expect(state.writes).toHaveLength(1);
  expect(state.turns).toEqual([]);
});

test("a direct chat link keeps its template through sign-in", async ({ page }) => {
  const state = await fixture(page, { signedIn: false });
  const destination = `/assistant?skill=${claims.slug}`;
  await page.goto(destination);
  await expect(page).toHaveURL(/\/login\?/);
  const login = new URL(page.url());
  expect(login.searchParams.get("return_to")).toBe(login.origin + destination);
  await page.getByRole("button", { name: "Continue with Google" }).click();
  await expect(page).toHaveURL(login.origin + destination);
  await expect(page.getByRole("heading", { name: "Audit research claims" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Open NyxBot" })).toBeEnabled();
  expect(state.turns).toEqual([]);
});

test("an enabled exact pin is reused after a temporary catalog error", async ({ page }) => {
  const state = await fixture(page, { existing: {}, previewError: 503 });
  await page.goto(`/assistant?skill=${research.slug}`);
  await expect(page.getByRole("alert")).toContainText("temporarily unavailable");
  expect(state.agentReads).toEqual([]);
  await page.getByRole("button", { name: "Retry verification" }).click();
  await page.getByRole("button", { name: "Open NyxBot" }).click();
  await expect(composerInput(page)).toContainText(research.slug);
  expect(state.writes).toEqual([]);
  expect(state.turns).toEqual([]);
});

for (const scenario of [
  { name: "denied access", options: { previewError: 403 }, message: "Check your Ornn access" },
  { name: "a typed pin mismatch", options: { previewError: 400 }, message: "could not verify" },
  { name: "a different digest", options: { badPreview: true }, message: "could not verify" },
]) {
  test(`a preview reports ${scenario.name} without offering Start`, async ({ page }) => {
    const state = await fixture(page, scenario.options);
    await page.goto(`/assistant?skill=${research.slug}`);
    await expect(page.getByRole("alert")).toContainText(scenario.message);
    await expect(page.getByRole("link", { name: "Choose another template" })).toBeVisible();
    await expect(page.getByRole("button", { name: "Open NyxBot" })).toHaveCount(0);
    expect(state.agentReads).toEqual([]);
    expect(state.turns).toEqual([]);
  });
}

test("a disabled pin and an unavailable engine keep the selection visible without starting", async ({ page }) => {
  const state = await fixture(page, { existing: { disabled: true } });
  await page.goto(`/assistant?skill=${research.slug}`);
  await page.getByRole("button", { name: "Open NyxBot" }).click();
  await expect(page.getByRole("alert")).toContainText("skill is disabled");
  const manage = await page.getByRole("link", { name: "Manage NyxBot skills" }).getAttribute("href");
  expect(new URL(manage!, page.url()).searchParams.get("agent")).toBe(agentId);
  expect(new URL(manage!, page.url()).searchParams.get("details")).toBe("skills");
  expect(state.writes).toEqual([]);
  expect(state.turns).toEqual([]);

  // Reload with a session without the NyxAgent engine.
  await page.unroute("**/api/v1/**");
  const unavailable = await fixture(page, { unavailable: true });
  await page.reload();
  await expect(page.getByRole("heading", { name: "Strengthen a research paper" })).toBeVisible();
  await expect(page.getByRole("status").filter({ hasText: "unavailable for this account" })).toBeVisible();
  expect(unavailable.previewRequests).toEqual([]);
});

for (const scenario of [
  { name: "a different version", options: { existing: { version: "1.0" } }, message: "different pin", manage: true },
  { name: "a full skills list", options: { skillCount: 16 }, message: "has 16 skills", manage: true },
  { name: "a specialist as the only available agent", options: { missingPersonalAgent: true }, message: "Could not load your personal NyxBot", manage: false },
]) {
  test(`a start stops for ${scenario.name}`, async ({ page }) => {
    const state = await fixture(page, scenario.options);
    await page.goto(`/assistant?skill=${research.slug}`);
    await page.getByRole("button", { name: "Open NyxBot" }).click();
    await expect(page.getByRole("alert")).toContainText(scenario.message);
    await expect(page.getByRole("link", { name: "Manage NyxBot skills" })).toHaveCount(Number(scenario.manage));
    expect(state.writes).toEqual([]);
    expect(state.turns).toEqual([]);
  });
}

test("a token-shaped deep link cannot attach a skill by itself", async ({ page }) => {
  const state = await fixture(page);
  const url = `/assistant?skill=${research.slug}&skillSetup=${"a".repeat(32)}`;
  await page.goto(url);
  await expect(page.getByRole("button", { name: "Open NyxBot" })).toBeEnabled();
  expect(state.agentReads).toEqual([]);
  expect(state.writes).toEqual([]);
  expect(state.turns).toEqual([]);
});

test("an expired setup reaches the selected preview without writing", async ({ page }) => {
  const state = await fixture(page, { signedIn: false });
  await page.goto("/testing");
  await openWelcome(page);
  await page.getByRole("button", { name: "Log in", exact: true }).click();
  await page.evaluate(() => {
    const key = "nyxid:skill-template-setup";
    const setup = JSON.parse(sessionStorage.getItem(key)!);
    sessionStorage.setItem(key, JSON.stringify({ ...setup, expiresAt: Date.now() - 1 }));
  });
  await page.getByRole("button", { name: "Continue with Google" }).click();
  await expect(page.getByRole("button", { name: "Open NyxBot" })).toBeEnabled();
  expect(state.agentReads).toEqual([]);
  expect(state.writes).toEqual([]);
  expect((await page.context().cookies()).filter(({ name }) => name.startsWith("nyxid_skill_setup_"))).toHaveLength(0);
});

test("a setup made in one account does not attach after the session changes", async ({ page }) => {
  const state = await fixture(page);
  await page.goto("/testing");
  await openWelcome(page);
  await expect(page.getByRole("button", { name: "Open NyxBot" })).toBeVisible();
  // Simulate the account changing between the welcome action and the return.
  const destination = await page.evaluate(() => {
    const skill = "research-paper-writing-rigor";
    const skillSetup = "a".repeat(32);
    sessionStorage.setItem("nyxid:skill-template-setup", JSON.stringify({
      owner: "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb", skill, skillSetup, expiresAt: Date.now() + 30_000,
    }));
    document.cookie = `nyxid_skill_setup_${skillSetup}=${skill}; Path=/; SameSite=Lax`;
    return `/assistant?${new URLSearchParams({ skill, skillSetup })}`;
  });
  await page.goto(destination);
  await expect(page.getByRole("button", { name: "Open NyxBot" })).toBeEnabled();
  expect(state.agentReads).toEqual([]);
  expect(state.writes).toEqual([]);
  expect((await page.context().cookies()).filter(({ name }) => name.startsWith("nyxid_skill_setup_"))).toHaveLength(0);
});

test("a rejected manual send retains the starter in the normal NyxBot composer", async ({ page }) => {
  const state = await fixture(page, { turnError: 429 });
  await page.goto(`/assistant?skill=${research.slug}`);
  await page.getByRole("button", { name: "Open NyxBot" }).click();
  await expect(composerInput(page)).toContainText(research.slug);
  await sendButton(page).click();
  await expect(page.getByText("Could not send your message")).toBeVisible();
  await expect(composerInput(page)).toContainText(research.slug);
  expect(state.writes).toHaveLength(1);
  expect(state.turns).toHaveLength(1);
  await sendButton(page).click();
  await expect(page.getByText("Please share your draft.")).toBeVisible();
  expect(state.writes).toHaveLength(1);
  expect(state.turns).toHaveLength(2);
});

for (const [name, destination] of [
  ["another route", "/dashboard"],
  ["an unknown template", "/assistant?skill=unknown"],
  ["a conflicting selector", `/assistant?skill=${research.slug}&draft=true`],
  ["a foreign origin", `https://example.invalid/assistant?skill=${research.slug}`],
]) {
  test(`a restored registration session does not hand off to ${name}`, async ({ page }) => {
    const profile = Promise.withResolvers<void>();
    await fixture(page, { profileGate: profile.promise });
    try {
      await page.goto(`/register?${new URLSearchParams({ return_to: destination })}`);
      await expect(page.getByRole("button", { name: "Continue with Google" })).toBeVisible();
    } finally {
      profile.resolve();
    }
    await expect(page.getByRole("link", { name: "NyxID" })).toHaveAttribute("href", "/dashboard");
    await expect(page).toHaveURL(/\/register\?/);
  });
}
