import { expect, test, type Page } from "@playwright/test";

const token = `nyx_pli_${"a".repeat(64)}`;
const bindingId = "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb";
const challengeId = "cccccccc-cccc-4ccc-8ccc-cccccccccccc";
const profileId = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
const threadId = `nyxa-${"d".repeat(32)}`;
const at = "2026-10-15T00:00:00Z";
const settings = {
  access_mode: "nyxmode",
  skip_destructive_confirmation: false,
  max_live_subagents: 2,
  max_concurrent_subagent_turns: 2,
  max_live_subagents_limit: 10,
  max_concurrent_subagent_turns_limit: 10,
};
const manager = {
  id: profileId,
  display_name: "@PlatformBot",
  username: "PlatformBot",
};
const sender = { id: "9007199254740993", display_name: null };
const origin = {
  binding_id: bindingId,
  binding_generation: 1,
  manager_authority_epoch: 1,
  personal_control_revision: 1,
};
const conversation = {
  id: threadId,
  title: "Personal Telegram",
  model: "nyxagent/chat",
  created_at: at,
  last_message_at: at,
  message_count: 2,
  active_turn: null,
  context_reset_at: null,
  agent: { id: "nyxbot", kind: "nyxbot", name: "NyxBot" },
  platform_thread: {
    platform: "telegram",
    manager_id: profileId,
    binding_id: bindingId,
    binding_generation: 1,
  },
};

async function fixture(page: Page, signedIn = true) {
  const state = {
    signedIn,
    timeOffset: 0,
    confirmed: false,
    revoked: false,
    reviews: 0,
    writes: [] as { path: string; body: unknown }[],
  };
  page.on("request", (request) => {
    expect(request.url()).not.toContain(token);
    expect(request.url()).not.toContain("/confirm%20");
  });
  await page.route("**/api/v1/**", async (route) => {
    const req = route.request();
    const path = new URL(req.url()).pathname.replace("/api/v1", "");
    let status = 200;
    let json: unknown = {};
    const expires_at = new Date(
      Date.now() + state.timeOffset + 600000,
    ).toISOString();
    if (req.method() !== "GET")
      state.writes.push({ path, body: req.postDataJSON() });
    if (path === "/users/me") {
      status = state.signedIn ? 200 : 401;
      json = state.signedIn
        ? {
            id: "owner",
            email: "owner@example.com",
            display_name: "Owner",
            is_active: true,
            is_admin: true,
            email_verified: true,
            created_at: at,
            capabilities: { enabled_features: ["assistant:nyxagent-engine"] },
          }
        : { error_code: 1001, message: "Sign in" };
    } else if (path === "/public/config")
      json = {
        invite_code_required: true,
        social_providers: [],
        telemetry_share_analytics: false,
        telemetry_dsn: null,
      };
    else if (path === "/platform-personal-links/preview")
      json = {
        purpose: "personal_nyxbot",
        status: "pending",
        manager,
        sender,
        expires_at: new Date(
          Date.now() + state.timeOffset + 86400000,
        ).toISOString(),
        invitation_required: true,
        requires_same_chat_confirmation: true,
      };
    else if (path === "/platform-personal-links/review") {
      state.reviews++;
      json = {
        challenge_id: challengeId,
        status: "review",
        expires_at,
        review_digest: `review-${state.reviews}`,
        account: { id: "owner", display_name: "Owner" },
        permission_mode: "nyxmode",
        billing: "personal",
        replaces_existing_binding: false,
        disclosures: {
          telegram_transport: true,
          existing_nyxbot_access: true,
          can_approve_sensitive_actions: true,
          started_work_may_continue_after_unlink: true,
        },
      };
    } else if (
      path === "/platform-personal-links/approve" ||
      path === "/platform-personal-links/status"
    )
      json = {
        challenge_id: challengeId,
        status: state.confirmed ? "confirmed" : "awaiting_chat_confirmation",
        expires_at,
        binding: state.confirmed ? { id: bindingId, generation: 1 } : null,
        confirmation_command: path.endsWith("approve")
          ? "/confirm P-ABCD-EFGH"
          : null,
      };
    else if (path === "/platform-personal-bindings")
      json = {
        bindings: [
          {
            id: bindingId,
            generation: 1,
            manager,
            sender,
            state: state.revoked ? "revoked" : "active",
            conversation_id: threadId,
            consented_at: at,
            revoked_at: state.revoked ? at : null,
          },
        ],
        next_before_id: null,
      };
    else if (path === `/platform-personal-bindings/${bindingId}/revoke`) {
      state.revoked = true;
      json = { revoked: true, work_may_continue_in_app: true };
    } else if (path === "/assistant/nyxagent/agents")
      json = {
        agents: [
          {
            id: "nyxbot",
            name: "NyxBot",
            kind: "nyxbot",
            status: "idle",
            created_by: "user",
            created_at: at,
            last_active_at: at,
            home_conversation_id: null,
          },
        ],
        limits: settings,
      };
    else if (path === "/assistant/nyxagent/conversations")
      json = { conversations: [conversation], next_cursor: null };
    else if (path === `/assistant/nyxagent/conversations/${threadId}`)
      json = {
        conversation,
        messages: [
          {
            id: "user-message",
            seq: 1,
            turn_id: "turn",
            role: "user",
            text: "My private Telegram question",
            status: "completed",
            error_code: null,
            created_at: at,
            platform_origin: origin,
          },
          {
            id: "reply",
            seq: 2,
            turn_id: "turn",
            role: "assistant",
            text: "Saved personal reply",
            status: "completed",
            error_code: null,
            created_at: at,
            platform_origin: origin,
          },
        ],
        before_seq: null,
      };
    else if (path === "/assistant/nyxagent/settings") json = settings;
    else if (path === "/assistant/nyxagent/groups") json = { groups: [] };
    else if (path === "/assistant/nyxagent/channels")
      json = { channel_agents: [] };
    else if (path === "/orgs") json = { organizations: [], orgs: [] };
    else if (path === "/channel-bots") json = { bots: [], total: 0 };
    else if (path === "/assistant/nyxagent/models")
      json = [{ id: "nyxagent/chat", label: "Chat", available: true }];
    else {
      status = 404;
      json = { error_code: 1, message: "Fixture route unavailable" };
    }
    await route.fulfill({ status, json });
  });
  return state;
}

test("fragment intent survives registration navigation without bypassing invitations or approving consent", async ({
  page,
}) => {
  const state = await fixture(page, false);
  await page.goto(`/nyxbot/link#intent_token=${token}`);
  await expect(page.getByText(/An invitation is required/)).toBeVisible();
  expect(new URL(page.url()).hash).toBe("");
  await page.getByRole("link", { name: "Register", exact: true }).click();
  expect(page.url()).not.toContain(token);
  await expect(page.getByPlaceholder("NYX-XXXXXXXX")).toBeVisible();
  await page.goto("/nyxbot/link");
  await expect(page.getByText("@PlatformBot", { exact: true })).toBeVisible();
  expect(state.writes.every((write) => write.path.endsWith("preview"))).toBe(
    true,
  );
  expect(
    await page.evaluate(() => localStorage.getItem("nyxid:personal-link:v1")),
  ).toBeNull();
});

test("one-time approval is never reconstructed after reload, then same-chat confirmation clears intent", async ({
  page,
}) => {
  const state = await fixture(page);
  await page.goto(`/nyxbot/link#intent_token=${token}`);
  await page.getByRole("button", { name: "Review personal access" }).click();
  await expect(
    page.getByText(/including existing memory and persona/),
  ).toBeVisible();
  await expect(page.getByText(/including destructive actions/)).toBeVisible();
  await page.getByRole("button", { name: "Approve personal access" }).click();
  await expect(
    page.getByText("/confirm P-ABCD-EFGH", { exact: true }),
  ).toBeVisible();
  expect(
    await page.evaluate(() => JSON.stringify({ localStorage, sessionStorage })),
  ).not.toContain("/confirm");
  await page.reload();
  await expect(page.getByText(/If you lost it, review again/)).toBeVisible();
  await expect(
    page.getByText("/confirm P-ABCD-EFGH", { exact: true }),
  ).toHaveCount(0);
  state.confirmed = true;
  await page.getByRole("button", { name: "Refresh status" }).click();
  await expect(page.getByText("Telegram is linked.")).toBeVisible();
  await expect
    .poll(() =>
      page.evaluate(() => sessionStorage.getItem("nyxid:personal-link:v1")),
    )
    .toBeNull();
});

test("saved personal Telegram thread stays accessible after owner disconnect", async ({
  page,
}) => {
  const state = await fixture(page);
  await page.goto(`/assistant?c=${threadId}`);
  await expect(
    page.getByText("My private Telegram question", { exact: true }),
  ).toBeVisible();
  await expect(
    page.getByText("Saved personal reply", { exact: true }),
  ).toBeVisible();
  await expect(
    page.getByText("via Telegram", { exact: true }).first(),
  ).toBeVisible();
  await page.goto("/assistant?panel=nyxbot");
  await expect(
    page.getByRole("link", { name: "Open saved Telegram thread" }),
  ).toBeVisible();
  await page
    .getByRole("button", { name: "Disconnect personal access" })
    .click();
  const dialog = page.getByRole("dialog");
  await expect(dialog).toContainText("Work already started may keep running");
  await dialog.getByRole("button", { name: "Disconnect", exact: true }).click();
  await expect(page.getByText(/9007199254740993 · Disconnected/)).toBeVisible();
  expect(
    state.writes.find((write) => write.path.endsWith("/revoke"))?.body,
  ).toEqual({ generation: 1 });
  await page.getByRole("link", { name: "Open saved Telegram thread" }).click();
  await expect(
    page.getByText("Saved personal reply", { exact: true }),
  ).toBeVisible();
});

test("an expired 10-minute review can be refreshed within the original 24-hour intent", async ({
  page,
}) => {
  const state = await fixture(page);
  await page.clock.install();
  await page.goto(`/nyxbot/link#intent_token=${token}`);
  await page.getByRole("button", { name: "Review personal access" }).click();
  await expect(
    page.getByRole("button", { name: "Approve personal access" }),
  ).toBeVisible();
  state.timeOffset = 601000;
  await page.clock.fastForward(601000);
  await expect(
    page.getByRole("button", { name: "Approve personal access" }),
  ).toHaveCount(0);
  await page.getByRole("button", { name: "Review again", exact: true }).click();
  await expect(
    page.getByRole("button", { name: "Approve personal access" }),
  ).toBeVisible();
  expect(state.reviews).toBe(2);
});
