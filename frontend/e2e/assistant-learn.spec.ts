import { expect, test, type Page } from "@playwright/test";
import { composerInput, sendButton } from "./helpers";

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

const catalogDenied = { status: 403, json: { error: "forbidden", message: "Skill unavailable through your Ornn access" } };
const pinMismatch = { status: 400, json: { error: "validation_error", details: { reason: "pinned_version_mismatch" } } };

async function fixture(page: Page, options: {
  unavailable?: boolean;
  previewError?: typeof catalogDenied | typeof pinMismatch;
  attachmentError?: typeof catalogDenied | typeof pinMismatch;
  skillCount?: number;
} = {}) {
  let revision = 2;
  let skills = Array.from({ length: options.skillCount ?? 1 }, (_, index) => ({
    ...other, skill_id: `22222222-2222-4222-8222-${String(index).padStart(12, "0")}`,
  }));
  const writes: { expected_revision: number; skills: typeof skills }[] = [];
  const turns: { agent_id?: string; conversation_id?: string; text: string }[] = [];
  const previewRequests: [string | null, string | null][] = [];
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
    if (path === "/users/me") return ok({
      id: owner, email: "owner@example.test", display_name: "Owner",
      capabilities: { enabled_features: options.unavailable ? [] : ["assistant:nyxagent-engine"] },
    });
    if (path === "/assistant/nyxagent/skills/catalog") {
      const skill = url.searchParams.get("skill");
      const version = url.searchParams.get("version");
      previewRequests.push([skill, version]);
      if (options.previewError) return route.fulfill(options.previewError);
      const template = [research, claims].find((entry) => entry.skillId === skill && entry.version === version);
      if (!template) return route.fulfill({ status: 404, json: {} });
      const sha256 = template.sha256;
      return ok({
        reference: { ...pin, skill_id: template.skillId, name: template.slug, version, sha256 },
        description: "Published skill",
        size_bytes: 400,
      });
    }
    if (path === "/assistant/nyxagent/agents") {
      return ok({ agents: [agent], limits });
    }
    if (path === "/assistant/nyxagent/settings") return ok(limits);
    if (path === `/assistant/nyxagent/agents/${agentId}/skills`) {
      if (request.method() === "PUT") {
        if (options.attachmentError) return route.fulfill(options.attachmentError);
        const selection = request.postDataJSON();
        writes.push(selection);
        expect(selection.expected_revision).toBe(revision);
        skills = selection.skills;
        revision += 1;
      }
      return ok({ revision, skills, metadata: {} });
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
      return ok({
        show_intro: false, suggestions_hidden: true, intro_version: 1,
        profile: { outcome: "skipped", topics: [], work_style: null, intro_version: 1, updated_at: now },
        topics: [], work_styles: [], recommendations: [],
      });
    }
    return route.fulfill({ status: 404, json: {} });
  });
  return { previewRequests, writes, turns };
}

test("Learn lets a returning owner browse by keyboard and send an edited starter without losing their draft", async ({ page }) => {
  const state = await fixture(page);
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
  await expect(composerInput(page)).toContainText("Draft copy, Source material, Claims to check");
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
  { name: "catalog denial", options: { previewError: catalogDenied }, message: "Check your Ornn access", action: "Choose another template" },
  { name: "a pinned-version mismatch during preview", options: { previewError: pinMismatch }, message: "could not verify the published package", action: "Choose another template" },
  { name: "a pinned-version mismatch during attachment", options: { attachmentError: pinMismatch }, message: "could not verify the published package", action: "Choose another template" },
  { name: "revoked catalog access during attachment", options: { attachmentError: catalogDenied }, message: "Check your Ornn access", action: "Choose another template" },
  { name: "the 16-skill limit", options: { skillCount: 16 }, message: "has 16 skills", action: "Manage NyxBot skills" },
]) {
  test(`Learn explains ${scenario.name} and links to recovery without opening a starter`, async ({ page }) => {
    const state = await fixture(page, { ...scenario.options });
    await page.goto("/assistant/learn");
    await page.getByRole("region", { name: "Research", exact: true })
      .getByRole("button", { name: "Try this · Audit research claims" }).click();
    await page.getByRole("dialog").getByRole("button", { name: "Open NyxBot" }).click();
    await expect(page.getByRole("alert")).toContainText(scenario.message);
    if (scenario.options.skillCount) {
      const link = page.getByRole("link", { name: scenario.action });
      const destination = new URL((await link.getAttribute("href"))!, page.url());
      expect(destination.pathname).toBe("/assistant");
      expect(destination.searchParams.get("agent")).toBe(agentId);
      expect(destination.searchParams.get("details")).toBe("skills");
    } else {
      await page.getByRole("button", { name: scenario.action }).click();
      await expect(page.getByRole("dialog")).toHaveCount(0);
      await expect(page).toHaveURL(/\/assistant\/learn$/);
    }
    await expect(composerInput(page)).toHaveCount(0);
    expect(state.writes).toEqual([]);
    expect(state.turns).toEqual([]);
  });
}

test("Learn shows the template starter with the NyxBot engine unavailable", async ({ page }) => {
  const state = await fixture(page, { unavailable: true });
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

test("Getting started reopens the existing workspace tour for a returning owner", async ({ page }) => {
  await fixture(page);
  await page.goto("/assistant/learn");
  await page.getByRole("link", { name: /Workspace tour/ }).click();
  await expect(page.getByRole("dialog", { name: "Ask NyxBot anything" })).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await page.reload();
  await expect(composerInput(page)).toBeVisible();
  await expect(page.getByRole("dialog")).toHaveCount(0);
});
