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
  callbackPath?: string;
  confirmName?: boolean;
  pauseAtProvider?: boolean;
  unavailable?: boolean;
  conflict?: boolean;
  badPreview?: boolean;
  launchError?: number;
  loseLaunchResponse?: boolean;
  queued?: boolean;
  outline?: boolean;
  skillCount?: number;
  existing?: Partial<typeof pin>;
  profileGate?: Promise<void>;
} = {}) {
  let signedIn = options.signedIn ?? true;
  let displayName = "Owner";
  const revision = 2;
  let conflict = options.conflict;
  const badPreview = options.badPreview;
  let launchError = options.launchError;
  let launched: { slug: string; state: "queued" | "admitted" | "cancelled" } | undefined;
  let loseLaunchResponse = options.loseLaunchResponse;
  const launches: { slug: string; setupId: string }[] = [];
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
      const template = [research, claims].find((entry) => entry.skillId === skill && entry.version === version);
      if (!template) return route.fulfill({ status: 404, json: {} });
      const sha256 = badPreview ? other.sha256 : template.sha256;
      return ok({
        reference: { ...pin, skill_id: template.skillId, name: template.slug, version, sha256 },
        description: "Published skill",
        size_bytes: 400,
        ...(options.outline ? {
          outline: {
            overview: "Review the argument before polishing the prose.",
            steps: [{ id: "argument", title: "Inspect the argument", text: "Identify missing evidence.", asks_you: true }],
          },
          author: { kind: "person", name: "Research Team" },
        } : {}),
      });
    }
    if (path === "/assistant/nyxagent/agents") {
      agentReads.push(path);
      return ok({ agents: [agent], limits });
    }
    if (path === "/assistant/nyxagent/settings") return ok(limits);
    if (path === `/assistant/nyxagent/agents/${agentId}/skills`) {
      if (request.method() === "PUT") writes.push(request.postDataJSON());
      return ok({ revision, skills, metadata: {} });
    }
    const launch = /^\/assistant\/nyxagent\/skill-templates\/([^/]+)\/start$/.exec(path);
    if (launch) {
      expect(request.method()).toBe("POST");
      const body = request.postDataJSON();
      expect(body).toEqual({ setup_id: expect.stringMatching(/^[0-9a-f]{32}$/) });
      launches.push({ slug: launch[1], setupId: body.setup_id });
      if (conflict || options.existing?.disabled || options.existing?.version || skills.length >= 16) {
        conflict = false;
        return route.fulfill({ status: 409, json: { error: "conflict", error_code: 1005, message: "Skills changed" } });
      }
      if (badPreview) {
        return route.fulfill({ status: 400, json: {
          error: "validation_error", error_code: 1003, message: "Package mismatch",
          details: { reason: "pinned_version_mismatch" },
        } });
      }
      if (launchError) {
        const status = launchError;
        launchError = undefined;
        return route.fulfill({ status, json: { error: "unavailable", error_code: 1005, message: "Launch unavailable" } });
      }
      launched ??= { slug: launch[1], state: options.queued ? "queued" : "admitted" };
      if (loseLaunchResponse) {
        loseLaunchResponse = false;
        return route.abort("failed");
      }
      return ok({ id: threadId });
    }
    if (path === `/assistant/nyxagent/conversations/${threadId}/stop`) {
      if (launched?.state === "queued") launched.state = "cancelled";
      return route.fulfill({ status: 204 });
    }
    if (path === "/assistant/nyxagent/turns") {
      turns.push(request.postDataJSON());
      return route.fulfill({
        headers: { "Content-Type": "text/event-stream", "X-NyxID-Conversation-Id": threadId },
        body: publishedReply,
      });
    }
    if (path === `/assistant/nyxagent/conversations/${threadId}`) {
      return ok({
        conversation: { ...conversation, gallery_skill: launched, pending_events: launched?.state === "queued" ? 1 : 0 },
        before_seq: null,
        messages: launched && launched.state !== "admitted" ? [] : [
          { id: "message-opening", seq: 1, turn_id: "turn-template", role: launched ? "event" : "user",
            text: launched ? "- Started a skill from the NyxBot template gallery." : "Earlier question",
            created_at: now, status: "completed", error_code: null },
          { id: "message-reply", seq: 2, turn_id: "turn-template", role: "assistant", text: "Please share your draft.", created_at: now, status: "completed", error_code: null },
        ],
      });
    }
    if (path === "/assistant/nyxagent/conversations") {
      return ok({ conversations: turns.length ? [conversation] : [], next_cursor: null });
    }
    if (path === "/assistant/nyxagent/groups") return ok({ groups: [] });
    if (path === "/assistant/nyxagent/onboarding") return ok({ show_intro: true });
    return route.fulfill({ status: 404, json: {} });
  });
  return { agentReads, previewRequests, socialRequests, writes, turns, launches, restoreSession: () => { signedIn = true; } };
}

async function openWelcome(page: Page, title = "Strengthen a research paper") {
  await page.getByRole("link", { name: `Try this · ${title}` }).click();
  const dialog = page.getByRole("dialog", { name: "Awesome, let’s get you started." });
  await expect(dialog).toBeVisible();
  await expect(dialog.getByRole("region", { name: "Selected template" }).getByRole("heading")).toHaveText(title);
}

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

test("new social accounts confirm their display name before the selected workflow starts", async ({ page }) => {
  const state = await fixture(page, { signedIn: false, confirmName: true });
  await page.goto("/testing?code=TEST-1234");
  await openWelcome(page);
  await page.getByRole("button", { name: "Create an account" }).click();
  await page.getByRole("button", { name: "Continue with Google" }).click();
  await expect(page.getByRole("heading", { name: "Choose your NyxID display name" })).toBeVisible({ timeout: 15_000 });
  expect(state.launches).toEqual([]);
  await page.getByRole("textbox", { name: "Display name" }).fill("New owner");
  await page.getByRole("button", { name: "Save and continue" }).click();
  await expect(page.getByText("Please share your draft.")).toBeVisible({ timeout: 15_000 });
  await expect(composerInput(page)).toBeEmpty();
  expect(state.launches[0]?.slug).toBe(research.slug);
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
    await expect(page.getByText("Please share your draft.")).toBeVisible({ timeout: 15_000 });
    const overview = page.getByRole("region", { name: "Skill overview" });
    await expect(overview.getByRole("heading", { name: "Starting template" })).toBeVisible();
    await expect(overview.getByText("Share the draft, outline, or sections you want reviewed.")).toBeVisible();
    await expect(composerInput(page)).toBeEmpty();
    if (path === "/register") expect(state.socialRequests[0]?.searchParams.get("invite_code")).toBe("TEST-1234");
    expect(state.launches).toEqual([{ slug: research.slug, setupId: returnTo.searchParams.get("skillSetup") }]);
    expect(state.writes).toEqual([]);
    expect(state.turns).toEqual([]);
    expect((await page.context().cookies()).find(({ name }) => name === cookie?.name)).toBeUndefined();

    await page.reload();
    await expect(page.getByText("Please share your draft.")).toBeVisible({ timeout: 15_000 });
    await expect(overview).toBeVisible();
    await expect(composerInput(page)).toBeEmpty();
    expect(state.launches).toHaveLength(1);
    await composerInput(page).fill("Here is my draft.");
    await sendButton(page).click();
    await expect.poll(() => state.turns[0]?.text).toBe("Here is my draft.");
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
  await expect(page.getByRole("region", { name: "Skill overview" }).getByRole("heading", { name: "Strengthen a research paper" })).toBeVisible({ timeout: 15_000 });
  await expect.poll(() => first.launches[0]?.slug).toBe(research.slug);
  await otherTab.getByRole("button", { name: "Continue with Google" }).click();
  await expect(otherTab.getByRole("region", { name: "Skill overview" }).getByRole("heading", { name: "Audit research claims" })).toBeVisible({ timeout: 15_000 });
  await expect.poll(() => second.launches[0]?.slug).toBe(claims.slug);
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
  await expect(page.getByRole("alert")).toContainText("Check its pinned version");
  expect(state.turns).toEqual([]);
  await page.getByRole("button", { name: "Try again" }).click();
  await expect(page.getByText("Please share your draft.")).toBeVisible();
  expect(state.launches).toHaveLength(2);
  expect(state.launches[0]).toEqual(state.launches[1]);
  expect(state.launches[0]?.slug).toBe(claims.slug);
  expect(state.writes).toEqual([]);
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
  await expect(page.getByText("Please share your draft.")).toBeVisible();
  expect(state.launches).toHaveLength(1);
  expect(state.turns).toEqual([]);
});

test("a direct chat link keeps its template through sign-in and waits for explicit Start", async ({ page }) => {
  const state = await fixture(page, { signedIn: false });
  const destination = `/assistant?skill=${claims.slug}`;
  await page.goto(destination);
  await expect(page).toHaveURL(/\/login\?/);
  const login = new URL(page.url());
  expect(login.searchParams.get("return_to")).toBe(login.origin + destination);
  await page.getByRole("button", { name: "Continue with Google" }).click();
  await expect(page).toHaveURL(login.origin + destination);
  await expect(page.getByRole("button", { name: "Start skill" })).toBeEnabled();
  expect(state.launches).toEqual([]);
  await page.getByRole("button", { name: "Start skill" }).click();
  await expect(page.getByText("Please share your draft.")).toBeVisible();
  expect(state.launches[0]?.slug).toBe(claims.slug);
  expect(state.turns).toEqual([]);
});

test("a reload after a lost launch response recovers the same conversation", async ({ page }) => {
  const state = await fixture(page, { loseLaunchResponse: true });
  await page.goto(`/assistant?skill=${research.slug}`);
  await page.getByRole("button", { name: "Start skill" }).click();
  await expect(page.getByRole("alert")).toContainText("Try again");
  await page.reload();
  await expect(page.getByText("Please share your draft.")).toBeVisible();
  expect(state.launches).toHaveLength(2);
  expect(state.launches[0]).toEqual(state.launches[1]);
  expect(state.turns).toEqual([]);
});

test("an account-bound legacy gallery URL starts in the regular chat", async ({ page }) => {
  const state = await fixture(page);
  const skillSetup = "b".repeat(32);
  await page.addInitScript(({ owner, skill, skillSetup }) => {
    sessionStorage.setItem("nyxid:skill-template-draft", JSON.stringify({ owner, skill, skillSetup }));
  }, { owner, skill: research.slug, skillSetup });
  await page.goto(`/assistant?skill=${research.slug}&skillSetup=${skillSetup}`);
  await expect(page.getByText("Please share your draft.")).toBeVisible();
  expect(state.launches).toEqual([{ slug: research.slug, setupId: skillSetup }]);
  expect(state.turns).toEqual([]);
});

test("a verified package outline and publisher appear inside the conversation", async ({ page }) => {
  const state = await fixture(page, { outline: true });
  await page.goto(`/assistant?skill=${research.slug}`);
  await page.getByRole("button", { name: "Start skill" }).click();
  await expect(page.getByText("Review the argument before polishing the prose.")).toBeVisible();
  await expect(page.getByText("By Research Team")).toBeVisible();
  await expect(page.getByRole("list", { name: "Steps" })).toContainText("Inspect the argument");
  await expect(composerInput(page)).toBeEmpty();
  expect(state.launches).toHaveLength(1);
  expect(state.turns).toEqual([]);
});

test("a queued launch survives reload and can be cancelled", async ({ page }) => {
  const state = await fixture(page, { queued: true });
  await page.goto(`/assistant?skill=${research.slug}`);
  await page.getByRole("button", { name: "Start skill" }).click();
  await expect(page.getByText("Waiting for NyxBot to start the skill…")).toBeVisible();
  await page.reload();
  await expect(page.getByText("Waiting for NyxBot to start the skill…")).toBeVisible();
  await page.getByRole("button", { name: "Cancel skill start" }).click();
  await expect(page.getByText("Skill start cancelled.")).toBeVisible();
  expect(state.launches).toHaveLength(1);
  expect(state.turns).toEqual([]);
});

for (const scenario of [
  { name: "denied access", options: { launchError: 403 }, message: "Check your Ornn access" },
  { name: "a different digest", options: { badPreview: true }, message: "could not verify" },
  { name: "a disabled pin", options: { existing: { disabled: true } }, message: "Check its pinned version" },
  { name: "a different version", options: { existing: { version: "1.0" } }, message: "Check its pinned version" },
  { name: "a full skills list", options: { skillCount: 16 }, message: "available space" },
]) {
  test(`a launch reports ${scenario.name}`, async ({ page }) => {
    const state = await fixture(page, scenario.options);
    await page.goto(`/assistant?skill=${research.slug}`);
    await page.getByRole("button", { name: "Start skill" }).click();
    await expect(page.getByRole("alert")).toContainText(scenario.message);
    expect(state.launches).toHaveLength(1);
    expect(state.writes).toEqual([]);
    expect(state.turns).toEqual([]);
  });
}

test("an unavailable engine keeps the selection visible", async ({ page }) => {
  const state = await fixture(page, { unavailable: true });
  await page.goto(`/assistant?skill=${research.slug}`);
  await expect(page.getByText("This template requires NyxBot")).toBeVisible();
  expect(state.launches).toEqual([]);
  expect(state.turns).toEqual([]);
});

test("a token-shaped deep link cannot launch a skill by itself", async ({ page }) => {
  const state = await fixture(page);
  await page.goto(`/assistant?skill=${research.slug}&skillSetup=${"a".repeat(32)}`);
  await expect(page.getByRole("button", { name: "Start skill" })).toBeEnabled();
  expect(state.launches).toEqual([]);
  expect(state.turns).toEqual([]);
});

test("an expired setup reaches the selected preview without launching", async ({ page }) => {
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
  await expect(page.getByRole("button", { name: "Start skill" })).toBeEnabled();
  expect(state.launches).toEqual([]);
  expect((await page.context().cookies()).filter(({ name }) => name.startsWith("nyxid_skill_setup_"))).toHaveLength(0);
});

test("a setup made in one account does not launch after the session changes", async ({ page }) => {
  const state = await fixture(page);
  await page.goto("/testing");
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
  await expect(page.getByRole("button", { name: "Start skill" })).toBeEnabled();
  expect(state.launches).toEqual([]);
  expect((await page.context().cookies()).filter(({ name }) => name.startsWith("nyxid_skill_setup_"))).toHaveLength(0);
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
