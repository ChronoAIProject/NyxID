import assert from "node:assert/strict";
import test from "node:test";

import {
  artifactBudgetDecision,
  artifactFileId,
  backoffDelay,
  choosePromptNavigation,
  classifyArtifactLink,
  decidePromptResume,
  daemonPath,
  decryptSessionEnvelope,
  installedDependencyVersion,
  isAuthFlowUrl,
  isTrustedArtifactUrl,
  markChatPageRecovered,
  modelItemMatches,
  modelLevelTargets,
  resolveNpmExecutable,
  sanitizeArtifactName,
  seedProfileName,
  shouldLeaveTabAlone,
  taskRecoveryDecision,
} from "./worker.mjs";

// Produced by Rust encrypt_login_snapshot for LOGIN_SNAPSHOT_FIXTURE_TOKEN.
// This cross-language wire fixture must never be regenerated silently.
const LOGIN_SNAPSHOT_FIXTURE_TOKEN = "nyx_owk_test-token-material";
const LOGIN_SNAPSHOT_FIXTURE = Buffer.from(
  '{"ciphertext_base64":"Wze5nFAvcqWAmAeGSzUc4agtvr4N9FH7L7CuwOTGynfBLTEV8ylIUuJ9gw7jJezU93XwSp8L6Q==","nonce_base64":"eo6ntLWmG/Ff7bVJ","salt_base64":"vJWSgK3y0bixEBTVH63n9xBLbUrXYY29SrxSWrZGg+c=","version":1}'
);

test("backoff is capped and jitter remains within the selected window", () => {
  assert.equal(backoffDelay(0, 100, 1000, () => 0), 50);
  assert.equal(backoffDelay(3, 100, 1000, () => 1), 800);
  assert.equal(backoffDelay(20, 100, 1000, () => 1), 1000);
});

test("artifact URLs are limited to ChatGPT content endpoints", () => {
  for (const url of [
    "https://chatgpt.com/backend-api/estuary/content?id=file-AbC123",
    "https://chat.openai.com/backend-api/files/file-AbC123",
    "https://files.oaiusercontent.com/file-AbC123/download",
    "blob:https://chatgpt.com/1234",
  ]) {
    assert.equal(isTrustedArtifactUrl(url), true, url);
  }
  for (const url of [
    "http://chatgpt.com/backend-api/files/file-1",
    "https://chatgpt.com.evil.example/backend-api/files/file-1",
    "https://example.com/?next=backend-api/file-1",
    "https://chatgpt.com/share/file-1",
    "blob:https://example.com/1234",
  ]) {
    assert.equal(isTrustedArtifactUrl(url), false, url);
  }
});

test("artifact links choose safe names and skip captured images", () => {
  const href = "https://chatgpt.com/backend-api/estuary/content?id=file-AbC123";
  assert.equal(artifactFileId(href), "file-abc123");
  assert.deepEqual(
    classifyArtifactLink({ href, download: "result.json", text: "Download" }),
    { href, name: "result.json", key: "file-abc123" }
  );
  assert.equal(
    classifyArtifactLink({ href, text: "result.json" }, [
      "https://files.oaiusercontent.com/file-AbC123/image.png",
    ]),
    null
  );
  assert.equal(
    classifyArtifactLink({ href: "https://example.com/file-1", text: "secret" }),
    null
  );
});

test("artifact names are bounded safe basenames", () => {
  assert.equal(sanitizeArtifactName("../private/result.json"), "_private_result.json");
  assert.equal(sanitizeArtifactName("..\\private\\result.json"), "_private_result.json");
  assert.equal(sanitizeArtifactName(".\u0000.", 3), "_");
  assert.equal(sanitizeArtifactName("../", 4), "_");
  assert.equal(Array.from(sanitizeArtifactName("x".repeat(200))).length, 128);
  assert.equal(sanitizeArtifactName("...", 7), "file_7");
});

test("artifact byte decisions enforce per-item and shared totals", () => {
  assert.equal(artifactBudgetDecision(0, 6, 5, 10), "skip");
  assert.equal(artifactBudgetDecision(4, 6, 10, 10), "accept");
  assert.equal(artifactBudgetDecision(5, 6, 10, 10), "stop");
  assert.equal(artifactBudgetDecision(0, 0, 10, 10), "skip");
});

test("a pre-send task may send only after the transcript is ready", () => {
  assert.deepEqual(
    decidePromptResume({
      phase: "claimed",
      prompt: "new question",
      turns: [],
      generating: false,
      transcriptReady: false,
    }),
    { action: "wait" }
  );
  assert.deepEqual(
    decidePromptResume({
      phase: "page_ready",
      prompt: "new question",
      turns: [],
      generating: false,
      transcriptReady: true,
    }),
    { action: "send" }
  );
});

test("recovery extracts a completed answer without resending", () => {
  assert.deepEqual(
    decidePromptResume({
      phase: "waiting_response",
      prompt: "What is 2 + 2?",
      turns: [
        { role: "user", text: "What is 2 + 2?" },
        { role: "assistant", text: "4" },
      ],
      generating: false,
      transcriptReady: true,
    }),
    { action: "complete", response: "4" }
  );
});

test("recovery waits when the sent prompt exists but has no settled answer", () => {
  assert.deepEqual(
    decidePromptResume({
      phase: "sent",
      prompt: "long answer",
      turns: [{ role: "user", text: "long answer" }],
      generating: true,
      transcriptReady: true,
    }),
    { action: "wait" }
  );
});

test("a persisted send attempt with no matching turn is never resent", () => {
  assert.deepEqual(
    decidePromptResume({
      phase: "send_attempted",
      prompt: "repeat me",
      turns: [],
      generating: false,
      transcriptReady: true,
    }),
    { action: "uncertain" }
  );
});

test("recovery preserves a live conversation when only the project root was persisted", () => {
  assert.deepEqual(
    choosePromptNavigation({
      recovering: true,
      phase: "send_attempted",
      isFollowup: false,
      currentUrl: "https://chatgpt.com/c/12345678-abcd",
      persistedUrl: "https://chatgpt.com/g/g-project/example/project",
      taskConversationUrl: null,
      requiredProjectUrl: "https://chatgpt.com/g/g-project/example/project",
    }),
    { error: null, target: null }
  );
});

test("pre-send recovery leaves an unrelated conversation before sending", () => {
  assert.deepEqual(
    choosePromptNavigation({
      recovering: true,
      phase: "page_ready",
      isFollowup: false,
      currentUrl: "https://chatgpt.com/c/12345678-abcd",
      persistedUrl: "https://chatgpt.com/g/g-project/example/project",
      taskConversationUrl: null,
      requiredProjectUrl: "https://chatgpt.com/g/g-project/example/project",
    }),
    {
      error: null,
      target: "https://chatgpt.com/g/g-project/example/project",
    }
  );
});

test("pre-send recovery without a conversation safely returns to the project", () => {
  assert.deepEqual(
    choosePromptNavigation({
      recovering: true,
      phase: "claimed",
      isFollowup: false,
      currentUrl: "https://chatgpt.com/",
      persistedUrl: null,
      taskConversationUrl: null,
      requiredProjectUrl: "https://chatgpt.com/g/g-project/example/project",
    }),
    {
      error: null,
      target: "https://chatgpt.com/g/g-project/example/project",
    }
  );
});

test("recovery navigates to a known conversation instead of an unrelated live tab", () => {
  assert.deepEqual(
    choosePromptNavigation({
      recovering: true,
      phase: "sent",
      isFollowup: true,
      currentUrl: "https://chatgpt.com/c/aaaaaaaa-bbbb",
      persistedUrl: "https://chatgpt.com/c/cccccccc-dddd",
      taskConversationUrl: "https://chatgpt.com/c/cccccccc-dddd",
      requiredProjectUrl: null,
    }),
    { error: null, target: "https://chatgpt.com/c/cccccccc-dddd" }
  );
});

test("a server-pinned conversation outranks a persisted project root", () => {
  assert.deepEqual(
    choosePromptNavigation({
      recovering: true,
      phase: "waiting_response",
      isFollowup: true,
      currentUrl: "https://chatgpt.com/",
      persistedUrl: "https://chatgpt.com/g/g-project/example/project",
      taskConversationUrl: "https://chatgpt.com/c/cccccccc-dddd",
      requiredProjectUrl: "https://chatgpt.com/g/g-project/example/project",
    }),
    { error: null, target: "https://chatgpt.com/c/cccccccc-dddd" }
  );
});

test("recovery fails closed when neither state nor the live tab identifies a conversation", () => {
  assert.deepEqual(
    choosePromptNavigation({
      recovering: true,
      phase: "send_attempted",
      isFollowup: false,
      currentUrl: "https://chatgpt.com/",
      persistedUrl: null,
      taskConversationUrl: null,
      requiredProjectUrl: null,
    }),
    { error: "recovery_conversation_unknown", target: null }
  );
});

test("the transcript baseline ignores an identical prompt from an older turn", () => {
  assert.deepEqual(
    decidePromptResume({
      phase: "send_attempted",
      prompt: "same prompt",
      turns: [
        { role: "user", text: "same prompt" },
        { role: "assistant", text: "old answer" },
      ],
      generating: false,
      transcriptReady: true,
      baselineTurnCount: 2,
    }),
    { action: "uncertain" }
  );
});

test("task recovery relaunches Chrome and then stops at a bounded threshold", () => {
  assert.deepEqual(
    taskRecoveryDecision({
      kind: "prompt",
      phase: "page_ready",
      failureCount: 3,
      maxFailures: 6,
      relaunchEvery: 3,
    }),
    { action: "recover", forceRelaunch: true }
  );
  assert.deepEqual(
    taskRecoveryDecision({
      kind: "prompt",
      phase: "page_ready",
      failureCount: 6,
      maxFailures: 6,
      relaunchEvery: 3,
    }),
    { action: "fail", code: "browser_recovery_exhausted" }
  );
});

test("post-send recovery exhaustion never authorizes a prompt retry", () => {
  assert.deepEqual(
    taskRecoveryDecision({
      kind: "prompt",
      phase: "send_attempted",
      failureCount: 6,
      maxFailures: 6,
      relaunchEvery: 3,
    }),
    { action: "fail", code: "prompt_delivery_uncertain" }
  );
  assert.deepEqual(
    taskRecoveryDecision({
      kind: "scrape",
      phase: "extracting",
      failureCount: 6,
      maxFailures: 6,
      relaunchEvery: 3,
    }),
    { action: "fail", code: "browser_recovery_exhausted" }
  );
});

test("a Rust login snapshot fixture decrypts with only its pool token", () => {
  const expected = { version: 1, cookies: [], origins: [] };

  assert.deepEqual(
    decryptSessionEnvelope(LOGIN_SNAPSHOT_FIXTURE, LOGIN_SNAPSHOT_FIXTURE_TOKEN),
    expected
  );
  assert.throws(
    () => decryptSessionEnvelope(LOGIN_SNAPSHOT_FIXTURE, "nyx_owk_wrong"),
    /session_decrypt_failed/
  );
});

test("idle Chrome recovery clears stale health errors", () => {
  const idle = {
    state: { current_task: null },
    health: { tab: 2 },
    chromeAlive: false,
    lastError: "cdp_connection_refused",
  };
  markChatPageRecovered(idle);
  assert.equal(idle.health.tab, 0);
  assert.equal(idle.chromeAlive, true);
  assert.equal(idle.lastError, null);

  const active = {
    state: { current_task: { task_id: "task-1" } },
    health: { tab: 2 },
    chromeAlive: false,
    lastError: "cdp_connection_refused",
  };
  markChatPageRecovered(active);
  assert.equal(active.lastError, "cdp_connection_refused");
});

test("login flow pages are recognised and left alone", () => {
  for (const url of [
    "https://auth.openai.com/authorize?x=1",
    "https://auth0.openai.com/u/login/identifier",
    "https://accounts.google.com/o/oauth2/v2/auth",
    "https://chatgpt.com/auth/login",
    "https://chatgpt.com/auth",
  ]) {
    assert.equal(isAuthFlowUrl(url), true, url);
    assert.equal(shouldLeaveTabAlone({ url, loggedIn: null }), true, url);
  }
  for (const url of ["https://chatgpt.com/", "https://chatgpt.com/c/abc", "https://example.com/auth/login", ""]) {
    assert.equal(isAuthFlowUrl(url), false, url);
  }
});

test("a logged-out ChatGPT tab is left alone until authenticated", () => {
  assert.equal(shouldLeaveTabAlone({ url: "https://chatgpt.com/", loggedIn: false }), true);
  assert.equal(shouldLeaveTabAlone({ url: "https://chatgpt.com/", loggedIn: true }), false);
  assert.equal(shouldLeaveTabAlone({ url: "https://chatgpt.com/", loggedIn: null }), false);
  // A lost page must still be recreated even if the last heartbeat saw logged out.
  assert.equal(shouldLeaveTabAlone({ url: undefined, loggedIn: false, pageOpen: false }), false);
  // A non-ChatGPT, non-auth page is steered back regardless of login state.
  assert.equal(shouldLeaveTabAlone({ url: "https://example.com/", loggedIn: false }), false);
});

test("npm resolves to the configured path, then node's sibling, then PATH", () => {
  assert.equal(resolveNpmExecutable({ configured: "/opt/npm", execPath: "/x/bin/node" }), "/opt/npm");
  assert.equal(
    resolveNpmExecutable({ execPath: "/x/bin/node", exists: (p) => p === "/x/bin/npm" }),
    "/x/bin/npm"
  );
  assert.equal(resolveNpmExecutable({ execPath: "/x/bin/node", exists: () => false }), "npm");
});

test("daemon PATH gains node's directory once", () => {
  assert.equal(daemonPath({ execPath: "/x/bin/node", envPath: "/usr/bin:/bin" }), "/x/bin:/usr/bin:/bin");
  assert.equal(daemonPath({ execPath: "/x/bin/node", envPath: "/x/bin:/usr/bin" }), "/x/bin:/usr/bin");
  assert.equal(daemonPath({ execPath: "/x/bin/node", envPath: "" }), "/x/bin:/usr/local/bin:/usr/bin:/bin");
});

test("installed dependency version is read from node_modules or null", () => {
  assert.equal(installedDependencyVersion("/i", () => JSON.stringify({ version: "1.62.1" })), "1.62.1");
  assert.equal(installedDependencyVersion("/i", () => { throw new Error("ENOENT"); }), null);
});

test("profile name is seeded only for a fresh profile", () => {
  const writes = [];
  const fs = {
    existsSync: () => false,
    mkdirSync: () => {},
    writeFileSync: (path, body) => writes.push([path, JSON.parse(body)]),
  };
  assert.equal(seedProfileName("/p", "NyxID Oracle w1", fs), true);
  assert.deepEqual(writes[0][1], { profile: { name: "NyxID Oracle w1" } });
  assert.equal(seedProfileName("/p", "x", { ...fs, existsSync: () => true }), false);
});

test("model labels map to ChatGPT reasoning levels with Pro first", () => {
  assert.equal(modelLevelTargets("chatgpt-5.5-pro")[0], "Pro");
  assert.equal(modelLevelTargets("gpt-5.5-extended")[0], "Pro");
  assert.equal(modelLevelTargets("Pro 扩展")[0], "Pro");
  assert.equal(modelLevelTargets("extra high")[0], "Extra High");
  assert.equal(modelLevelTargets("high")[0], "High");
  assert.equal(modelLevelTargets("balanced")[0], "Medium");
  assert.equal(modelLevelTargets("instant")[0], "Instant");
  assert.deepEqual(modelLevelTargets("custom-thing"), ["custom-thing"]);
  assert.deepEqual(modelLevelTargets(""), []);
});

test("level matching is exact before fuzzy so High never picks Extra High", () => {
  const high = modelLevelTargets("high");
  assert.equal(modelItemMatches("High", high, true), true);
  assert.equal(modelItemMatches("Extra High", high, true), false);
  assert.equal(modelItemMatches("Extra High", high, false), true);
  assert.equal(modelItemMatches("Pro", modelLevelTargets("chatgpt-5.5-pro"), true), true);
  assert.equal(modelItemMatches("Instant", modelLevelTargets("chatgpt-5.5-pro"), false), false);
});
