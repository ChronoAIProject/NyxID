import assert from "node:assert/strict";
import test from "node:test";

test("preferredModelPillIndex never selects an unlabelled control", () => {
  // The composer region also contains icon-only menu buttons such as
  // composer-plus-btn ("Add files and more"), whose innerText is empty.
  // Returning index 0 there makes the worker click the wrong button and no
  // model menu opens (menu_not_opened, items=0).
  assert.equal(preferredModelPillIndex(["", "6\nPro"]), 1);
  assert.equal(preferredModelPillIndex(["", "Some Control"]), 1);
  assert.equal(preferredModelPillIndex(["", ""]), -1);
  assert.equal(preferredModelPillIndex([]), -1);
  // A recognised level still wins outright.
  assert.equal(preferredModelPillIndex(["", "GPT-5.5 High"]), 1);
});


import {
  PRE_SEND_ACTION_MS,
  composerHasDraft,
  draftNeedsFastClear,
  COMPOSER_FAST_CLEAR_CHARS,
  composerVisibleHitPoint,
  retryPresendModelRead,
  usageCooldownConfig,
  effortSelectionMismatch,
  chooseSwitcherFamilyEntry,
  accountFingerprint,
  artifactBudgetDecision,
  artifactFileId,
  backoffDelay,
  chooseChatPage,
  choosePromptNavigation,
  classifyArtifactLink,
  decidePromptResume,
  daemonPath,
  decryptSessionEnvelope,
  encryptSessionEnvelope,
  validateSessionSnapshot,
  canImportLogin,
  savedLoginDecision,
  installedDependencyVersion,
  invalidateSavedLogin,
  isAccountChangeUrl,
  isAuthFlowUrl,
  isTrustedArtifactUrl,
  markChatPageRecovered,
  modelItemMatches,
  chooseNestedLevelEntry,
  reportedPromptModel,
  modelSelectionDetail,
  preferredModelPillIndex,
  modelSelectionDiagnostics,
  formatPickerLabels,
  requireInteractionRead,
  modelSelectionFailureReason,
  modelLevelTargets,
  pillShowsLevel,
  splitTierOffered,
  detectPillLevel,
  dropCancelledCommand,
  resolveNpmExecutable,
  sanitizeArtifactName,
  seedProfileName,
  shouldLeaveTabAlone,
  taskRecoveryDecision,
  workerCapabilities,
} from "./worker.mjs";

test("member enrollment retains task and control capabilities without offering pool-token login imports", () => {
  const own = workerCapabilities(`nyx_owi_${"a".repeat(64)}`);
  assert.deepEqual(own, ["commands_v1", "upgrade_v1", "attempt_fencing_v1"]);
  const legacy = workerCapabilities(`nyx_owk_${"a".repeat(64)}`);
  for (const capability of [...own, "session_import_v1", "saved_login_v1"]) {
    assert.ok(legacy.includes(capability));
  }
});

test("saved account fingerprints bind both identity and pool without storing upstream identifiers", () => {
  const fingerprint = accountFingerprint("synthetic-account-a", "synthetic-pool-token");
  assert.match(fingerprint, /^[a-f0-9]{64}$/);
  assert.equal(fingerprint, accountFingerprint("synthetic-account-a", "synthetic-pool-token"));
  assert.notEqual(fingerprint, accountFingerprint("synthetic-account-b", "synthetic-pool-token"));
  assert.notEqual(fingerprint, accountFingerprint("synthetic-account-a", "another-pool-token"));
  assert.equal(fingerprint.includes("synthetic-account-a"), false);
});

test("account-change detection excludes normal session refresh and unrelated hosts", () => {
  for (const url of ["https://chatgpt.com/api/auth/signout", "https://chatgpt.com/auth/login", "https://auth.openai.com/logout", "https://chat.openai.com/switch-account"]) {
    assert.equal(isAccountChangeUrl(url), true, url);
  }
  assert.equal(isAccountChangeUrl("https://accounts.google.com/", true), true);
  for (const url of ["https://chatgpt.com/api/auth/session", "https://auth.openai.com/api/auth/refresh", "https://chatgpt.com/backend-api/conversation", "https://unrelated.example/login", "https://chatgpt.com.evil.example/logout", "invalid"]) {
    assert.equal(isAccountChangeUrl(url), false, url);
  }
  assert.equal(isAccountChangeUrl("https://accounts.google.com/", false), false);
});

test("observed account changes revoke publication authority while retaining the original fingerprint", () => {
  const state = { saved_login: { status: "verified", account_fingerprint: "original", source_revision: "revision", pending_publication_id: "publication" } };
  assert.equal(invalidateSavedLogin(state, "untrusted"), true);
  assert.equal(state.saved_login.pending_publication_id, null);
  assert.equal(state.saved_login.account_fingerprint, "original");
  assert.equal(state.saved_login.source_revision, "revision");
  assert.equal(invalidateSavedLogin(state, "external_login"), true);
  assert.equal(invalidateSavedLogin(state, "untrusted"), false);
  assert.equal(state.saved_login.status, "external_login");
  assert.equal(invalidateSavedLogin({}, "external_login"), false);
  assert.equal(invalidateSavedLogin({ saved_login: { status: "importing" } }, "external_login"), false);
});

test("saved login import fences every post-send phase and permits only logged-out pre-send recovery", () => {
  for (const phase of ["send_attempted", "sent", "waiting_response", "settling", "scraping", "unknown"]) {
    assert.equal(canImportLogin({ current_task: { phase } }, false), false, phase);
  }
  for (const phase of ["claimed", "page_ready", "ready_to_send"]) {
    assert.equal(canImportLogin({ current_task: { phase } }, false), true);
    assert.equal(canImportLogin({ current_task: { phase } }, true), false);
  }
  assert.equal(canImportLogin({}, true), true);
});

test("saved login tracks imported revision separately from sibling publications and human generation", () => {
  const now = Date.now();
  const desired = { status: "available", profile: { id: "profile", generation: "human-1", revision: "revision-2", updated_at: new Date(now).toISOString() },
    binding: { binding_id: "binding", replace_existing: false } };
  const state = { saved_login: { profile_id: "profile", binding_id: "binding", generation: "human-1",
    source_revision: "revision-1", attempted_revision: "revision-1", status: "verified" } };
  assert.equal(savedLoginDecision({}, desired, true), "preserve_existing");
  assert.equal(savedLoginDecision({}, desired, false), "import");
  assert.equal(savedLoginDecision(state, desired, true), "sibling_revision");
  assert.equal(state.saved_login.source_revision, "revision-1");
  assert.equal(savedLoginDecision(state, desired, true, now + 16 * 60 * 1000), "import");
  assert.equal(savedLoginDecision({ ...state, current_task: { phase: "sent" } }, desired, false), "defer");
  assert.equal(savedLoginDecision(state, { ...desired, profile: { ...desired.profile, generation: "human-2" } }, true), "import");
  state.saved_login.source_revision = "revision-2";
  assert.equal(savedLoginDecision(state, desired, true), "refresh");
  state.saved_login.status = "untrusted";
  assert.equal(savedLoginDecision(state, desired, true), "preserve_existing");
  assert.equal(savedLoginDecision(state, desired, false), "import");
  state.saved_login.status = "external_login";
  assert.equal(savedLoginDecision(state, desired, true), "preserve_existing");
  assert.equal(savedLoginDecision(state, desired, false), "preserve_existing");
  assert.equal(savedLoginDecision(state, { ...desired, profile: { ...desired.profile, generation: "human-2" } }, true), "import");
  state.saved_login.status = "failed";
  state.saved_login.attempted_revision = "revision-2";
  assert.equal(savedLoginDecision(state, desired, false), "failed_revision");
  assert.equal(savedLoginDecision(state, { ...desired, profile: { ...desired.profile, revision: "revision-3" } }, true), "import");
  assert.equal(savedLoginDecision(state, { ...desired, status: "token_changed" }, true), "unavailable");
});

test("worker export envelope authenticates token, bytes, size and version", () => {
  const snapshot = { version: 1, cookies: [], origins: [] };
  const encrypted = encryptSessionEnvelope(snapshot, "synthetic-token");
  assert.deepEqual(decryptSessionEnvelope(encrypted, "synthetic-token"), snapshot);
  assert.throws(() => decryptSessionEnvelope(encrypted, "wrong-token"), /session_decrypt_failed/);
  const changed = JSON.parse(encrypted);
  const bytes = Buffer.from(changed.ciphertext_base64, "base64");
  bytes[0] ^= 1;
  changed.ciphertext_base64 = bytes.toString("base64");
  assert.throws(() => decryptSessionEnvelope(Buffer.from(JSON.stringify(changed)), "synthetic-token"), /session_decrypt_failed/);
  assert.throws(() => encryptSessionEnvelope({ data: "x".repeat(360000) }, "synthetic-token"), /session_plaintext_too_large/);
});

test("login snapshots validate cookies and storage before an importer mutates browser state", () => {
  const cookie = { name: "session", value: "synthetic", domain: "auth.openai.com", path: "/api/auth",
    expires: -1, secure: true, httpOnly: true, sameSite: "Lax" };
  const valid = validateSessionSnapshot({ version: 1, cookies: [cookie, { ...cookie, domain: "unrelated.example" }], origins: [] });
  assert.deepEqual(valid.cookies, [cookie]);
  assert.throws(() => validateSessionSnapshot({ version: 1, cookies: [{ ...cookie, domain: "unrelated.example" }] }), /session_snapshot_no_cookies/);
  assert.throws(() => validateSessionSnapshot({ version: 1, cookies: [{ ...cookie, path: "invalid" }] }), /session_snapshot_invalid/);
  assert.throws(() => validateSessionSnapshot({ version: 1, cookies: [cookie], origins: [{ origin: "https://chatgpt.com", local_storage: "invalid" }] }), /session_snapshot_invalid/);
});

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
  assert.deepEqual(modelLevelTargets("chatgpt-6-pro"), ["Pro", "Pro Extended", "Pro 扩展", "扩展"]);
  assert.deepEqual(modelLevelTargets("chatgpt-6-pro-extended"), ["Pro", "Pro Extended", "Pro 扩展", "扩展"]);
  assert.deepEqual(modelLevelTargets("Pro 扩展"), ["Pro", "Pro Extended", "Pro 扩展", "扩展"]);
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
  assert.equal(modelItemMatches("Extra High", high, false), false);
  assert.equal(modelItemMatches("Pro", modelLevelTargets("chatgpt-5.5-pro"), true), true);
  assert.equal(modelItemMatches("Instant", modelLevelTargets("chatgpt-5.5-pro"), false), false);
});

test("pill text verifies the selected level without cross-matching", () => {
  const pro = modelLevelTargets("chatgpt-5.5-pro");
  assert.equal(pillShowsLevel("GPT-5.5 Pro", pro), true);
  assert.equal(pillShowsLevel("Pro", pro), true);
  assert.equal(pillShowsLevel("GPT-5.5 Instant", pro), false);
  const high = modelLevelTargets("high");
  assert.equal(pillShowsLevel("High", high), true);
  assert.equal(pillShowsLevel("Extra High", high), false);
  assert.equal(pillShowsLevel("", high), false);
});

test("an oversized leftover draft takes the single-assignment clear", () => {
  // fill("") drives a select-all and delete through React; on a composer
  // holding tens of thousands of characters that does not finish inside
  // PRE_SEND_ACTION_MS. The clear is best-effort, so the timeout is swallowed,
  // the draft survives a browser relaunch, and every worker that picks the task
  // up dies as operation_timeout@selecting_model. Observed 2026-09-21 with an
  // 83,046 character draft that walked through a 15-worker pool.
  assert.equal(draftNeedsFastClear(83046), true);
  assert.equal(draftNeedsFastClear(COMPOSER_FAST_CLEAR_CHARS), true);
  // An ordinary leftover still takes the normal path.
  assert.equal(draftNeedsFastClear(COMPOSER_FAST_CLEAR_CHARS - 1), false);
  assert.equal(draftNeedsFastClear(12), false);
  assert.equal(draftNeedsFastClear(0), false);
  // A missing or unreadable length must never trigger it.
  assert.equal(draftNeedsFastClear(undefined), false);
  assert.equal(draftNeedsFastClear(NaN), false);
  assert.equal(draftNeedsFastClear(Infinity), false);
  // The emptiness check itself is unchanged.
  assert.equal(composerHasDraft("  "), false);
  assert.equal(composerHasDraft("Architec"), true);
});

test("a menu without the split tier is not a missing level", () => {
  // Captured from a live Pro account 2026-09-21. The composer pill's menu now
  // lists model versions; "6 Pro" is a heading with aria-checked unset, so it
  // cannot be clicked. modelLevelTargets still asks for Pro Extended on every
  // Pro request, so the hunt can never succeed - the pool went fully down with
  // every task rerouted through all 15 workers before failing level_unavailable.
  const liveMenu = [
    { text: "6 Pro" }, { text: "" }, { text: "Latest" },
    { text: "GPT-5.6 Sol" }, { text: "GPT-5.5 Leaving on October 14" },
  ];
  assert.equal(splitTierOffered(liveMenu), false);
  // Where the split does exist, nothing changes: the hunt still applies.
  assert.equal(splitTierOffered([{ text: "Pro Standard" }, { text: "Pro Extended" }]), true);
  assert.equal(splitTierOffered([{ text: "Pro \u6269\u5c55" }]), true);
  assert.equal(splitTierOffered([{ text: "Pro \u6807\u51c6" }]), true);
  // A bare Pro entry is not a split tier.
  assert.equal(splitTierOffered([{ text: "Pro" }, { text: "GPT 6 Pro" }]), false);
  // Absent or malformed menus never claim the split.
  assert.equal(splitTierOffered([]), false);
  assert.equal(splitTierOffered(undefined), false);
  assert.equal(splitTierOffered([{}, { text: null }]), false);
});

test("pill level detection prefers the longest alias", () => {
  assert.equal(detectPillLevel("GPT-5.5 Extra High"), "Extra High");
  assert.equal(detectPillLevel("GPT-5.5 High"), "High");
  assert.equal(detectPillLevel("Pro 扩展"), "Pro");
  assert.equal(detectPillLevel("GPT-5.5"), null);

  // Observed live on chatgpt.com 2026-09-16: the composer pill renders the
  // family and the level as separate text nodes, so innerText is "6\nPro".
  // First-line-only reads "6", strips it as a version, and reports the pill
  // as unrecognized, after which the worker clicks the wrong control.
  assert.equal(detectPillLevel("6\nPro"), "Pro");
  assert.equal(detectPillLevel("GPT-6\nPro"), "Pro");
  // "Thinking" is not in MODEL_LEVELS, so it is null on one line or two;
  // the multi-line path must not invent a level that single-line lacks.
  assert.equal(detectPillLevel("6\nThinking"), detectPillLevel("6 Thinking"));
  // A single-line label must behave exactly as before.
  assert.equal(detectPillLevel("6 Pro"), "Pro");
  // A menu entry whose second line is a description must not be folded in.
  assert.equal(detectPillLevel("GPT-5.5"), null);
});

test("a cancelled pending command is dropped and draining is recomputed", () => {
  const runtime = { state: { pending_command: { id: "c1", command: "upgrade" }, draining: true, drain_requested: false } };
  assert.equal(dropCancelledCommand(runtime, ["c1"]), true);
  assert.equal(runtime.state.pending_command, null);
  assert.equal(runtime.state.draining, false);
  const drained = { state: { pending_command: { id: "c2", command: "restart" }, draining: true, drain_requested: true } };
  assert.equal(dropCancelledCommand(drained, ["c2"]), true);
  assert.equal(drained.state.draining, true);
  const untouched = { state: { pending_command: { id: "c3", command: "restart" }, draining: true } };
  assert.equal(dropCancelledCommand(untouched, ["other"]), false);
  assert.equal(untouched.state.pending_command.id, "c3");
});

test("one ChatGPT tab is driven and duplicates are reported for closing", () => {
  const page = (url) => ({ url: () => url });
  const a = page("https://chatgpt.com/c/one");
  const b = page("https://chatgpt.com/");
  const login = page("https://auth.openai.com/authorize");
  const blank = page("about:blank");
  assert.deepEqual(chooseChatPage([blank, a, b]), { chosen: a, duplicates: [b], navigate: false });
  assert.deepEqual(chooseChatPage([login, blank]), { chosen: login, duplicates: [], navigate: false });
  assert.deepEqual(chooseChatPage([blank]), { chosen: blank, duplicates: [], navigate: true });
  assert.deepEqual(chooseChatPage([]), { chosen: null, duplicates: [], navigate: true });
});


test("6-era pill labels detect the level without adding locale aliases", () => {
  for (const label of ["GPT-6 Pro", "6 Pro", "Pro", "Pro 扩展", "扩展"]) {
    assert.equal(detectPillLevel(label), "Pro", label);
    assert.equal(modelLevelTargets(label)[0], "Pro", label);
    assert.equal(pillShowsLevel(label, modelLevelTargets("chatgpt-6-pro")), true, label);
  }
  for (const label of ["Extra High", "GPT-6 Extra High", "6-extra-high", "超高"]) {
    assert.equal(detectPillLevel(label), "Extra High", label);
  }
  for (const label of ["6", "GPT-6", "Auto", "自动", "Profile", "Products", "Highlight", "Improve"] ) {
    assert.equal(detectPillLevel(label), null, label);
  }
});

test("fuzzy level matching rejects partial labels and conflicting canonical levels", () => {
  const high = modelLevelTargets("high");
  assert.equal(modelItemMatches("GPT-6 High", high, false), true);
  assert.equal(modelItemMatches("GPT-6 Extra High", high, false), false);
  for (const label of ["P", "Profile", "Improve"]) {
    assert.equal(modelItemMatches(label, modelLevelTargets("pro"), false), false, label);
  }
});

test("nested entry choice prefers exact target, then fuzzy target, then a recognized checked level", () => {
  const targets = modelLevelTargets("chatgpt-6-pro");
  const items = [{ text: "Instant", checked: true }, { text: "GPT-6 Pro" }, { text: "Pro" }];
  assert.equal(chooseNestedLevelEntry(items, targets), 2);
  assert.equal(chooseNestedLevelEntry(items.slice(0, 2), targets), 1);
  assert.equal(chooseNestedLevelEntry(items.slice(0, 1), targets), -1);
  assert.equal(chooseNestedLevelEntry(items.slice(0, 1), targets, false), -1);
});

test("split Pro tiers choose Extended by default and Standard only explicitly", () => {
  const tiers = [{ text: "Instant" }, { text: "Extra High" }, { text: "Pro Extended" }, { text: "Pro Standard" }];
  assert.equal(chooseNestedLevelEntry(tiers, modelLevelTargets("chatgpt-6-pro")), 2);
  assert.equal(chooseNestedLevelEntry(tiers, modelLevelTargets("chatgpt-6-pro-extended")), 2);
  assert.equal(chooseNestedLevelEntry(tiers, modelLevelTargets("Pro 扩展")), 2);
  // A single "Pro" entry still wins exactly for both labels.
  const single = [{ text: "Instant" }, { text: "Pro" }];
  assert.equal(chooseNestedLevelEntry(single, modelLevelTargets("chatgpt-6-pro")), 1);
  assert.equal(chooseNestedLevelEntry(single, modelLevelTargets("chatgpt-6-pro-extended")), 1);
  // Only an Extended entry present: the plain label falls back to it fuzzily; never Instant.
  assert.equal(chooseNestedLevelEntry([{ text: "Instant" }, { text: "Pro Extended" }], modelLevelTargets("chatgpt-6-pro")), 1);
  for (const label of ["chatgpt-6-pro", "chatgpt-6-pro-extended"]) {
    assert.equal(pillShowsLevel("GPT-6 Pro Standard", modelLevelTargets(label)), false, label);
    assert.equal(pillShowsLevel("GPT-6 Pro Extended", modelLevelTargets(label)), true, label);
    assert.equal(modelSelectionDetail({ level: modelLevelTargets(label)[0], verified: true, reason: "selected" }), "selected=Pro");
  }
});

test("nested entry choice never picks an unchecked first item or a checked arbitrary action", () => {
  const targets = modelLevelTargets("chatgpt-6-pro");
  for (const items of [[], [{ text: "Instant" }], [{ text: "Delete conversation", checked: true }],
    [{ text: "Profile", checked: true }], [{ text: "自动", checked: true }]]) {
    assert.equal(chooseNestedLevelEntry(items, targets), -1);
  }
  assert.equal(chooseNestedLevelEntry([{ text: "Extra High" }], modelLevelTargets("high")), -1);
});

test("prompt result retains the request; observations are separate canonical metadata", () => {
  for (const verified of [true, false]) {
    assert.equal(reportedPromptModel({ model: "chatgpt-6-pro", model_selected: "GPT-6 Pro", verified }), "chatgpt-6-pro");
    assert.equal(reportedPromptModel({ model: "chatgpt-6-pro", model_selected: "自动", verified }), "chatgpt-6-pro");
  }
  for (const model_selected of [undefined, null, ""]) {
    assert.equal(reportedPromptModel({ model: "chatgpt-6-pro", model_selected, clicked: "Pro" }), "chatgpt-6-pro");
  }
});

test("selection phase detail uses only canonical levels and stable outcome codes", () => {
  assert.equal(modelSelectionDetail({ level: "Pro", verified: true, reason: "selected" }), "selected=Pro");
  assert.equal(modelSelectionDetail({ level: "Extra High", verified: false, reason: "unverified" }), "unverified=Extra High");
  assert.equal(modelSelectionDetail({ level: "private custom label", verified: false, reason: "unverified" }), "unverified=custom");
  for (const reason of ["timeout", "picker_unavailable", "level_unavailable", "selection_failed"]) {
    assert.equal(modelSelectionDetail({ level: "Pro", observed: "private page text", reason }), reason);
  }
  assert.equal(modelSelectionDetail({ level: "Pro", verified: true, reason: "timeout" }), "timeout");
});


test("pill preference uses canonical levels before legacy labels in either candidate source", () => {
  assert.equal(preferredModelPillIndex(["GPT tools", "Instant"]), 1);
  assert.equal(preferredModelPillIndex(["Search", "GPT-6"]), 1);
  assert.equal(preferredModelPillIndex(["Tools", "自动"]), 0);
  assert.equal(preferredModelPillIndex(["自动"]), 0);
  assert.equal(preferredModelPillIndex(["Tools", "6 Pro", "High"]), 1);
  assert.equal(preferredModelPillIndex([]), -1);
});

test("selection diagnostics contain structural metadata and canonical levels without raw labels", () => {
  const observed = "private pill marker";
  const output = modelSelectionDiagnostics({ pill: { structural: true }, observed,
    items: [{ text: "Instant" }, { text: "Pro with private hint" }, { text: "Pro" }, { text: "private action" }] });
  assert.equal(output, "pill_source=structural pill_level=unrecognized pill_text_length=19 items=4 recognized=[Instant,Pro]");
  assert.ok(!output.includes("private"));
  assert.match(modelSelectionDiagnostics({ pill: { structural: false }, observed: "6 Pro" }),
    /pill_source=fallback pill_level=Pro pill_text_length=5 items=0 recognized=\[\]/);
  assert.equal(modelSelectionDiagnostics(null),
    "pill_source=none pill_level=unrecognized pill_text_length=0 items=0 recognized=[]");
});

test("picker label formatting bounds labels and items, JSON-escapes controls, and accepts an empty snapshot", () => {
  for (const snapshot of [undefined, null, {}]) {
    assert.equal(formatPickerLabels(snapshot), 'picker_labels pill="" items=[]');
  }
  const parse = (line) => {
    const [, pill, items] = /^picker_labels pill=(.+) items=(.+)$/.exec(line);
    return { pill: JSON.parse(pill), items: JSON.parse(items) };
  };
  const controls = '自动\n\r\t"\\\u0000\u001b\u2028\u2029';
  const encoded = formatPickerLabels({ observed: controls, items: [{ text: controls }] });
  assert.doesNotMatch(encoded, /[\n\r\t\u0000\u001b\u2028\u2029]/);
  assert.deepEqual(parse(encoded), { pill: controls, items: [controls] });
  const bounded = parse(formatPickerLabels({
    observed: "x".repeat(39) + "🧭discard",
    items: Array.from({ length: 30 }, (_, index) => ({ text: "item-" + index + ":" + "y".repeat(50) })),
  }));
  assert.equal(bounded.pill, "x".repeat(39) + "🧭");
  assert.equal(bounded.items.length, 24);
  assert.ok(bounded.items.every((label) => label.length === 40));
  assert.equal(bounded.items[0], "item-0:" + "y".repeat(33));
  assert.equal(bounded.items[23], "item-23:" + "y".repeat(32));
});

test("null browser reads carry the interaction deadline code instead of causing a TypeError", () => {
  assert.throws(() => requireInteractionRead(null), { message: "interaction_deadline", code: "interaction_deadline" });
  for (const value of [false, 0, "", { open: false }, { clear: true }]) {
    assert.equal(requireInteractionRead(value), value);
  }
});

test("selection distinguishes the shared deadline from a shorter step timeout", () => {
  const error = Object.assign(new Error("timed out"), { name: "TimeoutError" });
  assert.equal(modelSelectionFailureReason(error, { deadline: 100, aborted: false }, 50), "selection_failed");
  assert.equal(modelSelectionFailureReason(error, { deadline: 100, aborted: false }, 100), "timeout");
  assert.equal(modelSelectionFailureReason(error, { deadline: 100, aborted: true }, 50), "timeout");
  assert.equal(modelSelectionFailureReason({ code: "interaction_deadline" }, { deadline: 100, aborted: false }, 50), "interaction_deadline");
});

import { failureDetail, cooldownRemaining, classifyChatGptError, stableErrorCode,
  switcherMetadata, switcherMatches, compactModelLabel, chooseSwitcherEntry, effortMetadata } from './worker.mjs';

test('failure attribution is bounded metadata and classifies real crash messages', () => {
  assert.equal(stableErrorCode(new Error('Target crashed')), 'page_crashed');
  assert.equal(stableErrorCode(new Error('Page crashed')), 'page_crashed');
  assert.equal(stableErrorCode(new Error('locator.waitFor: Timeout 60000ms exceeded')), 'operation_timeout');
  assert.equal(failureDetail('composer_not_found', 'page_ready'), 'composer_not_found@page_ready');
  assert.equal(failureDetail('page_crashed', 'waiting_response'), 'page_crashed@waiting_response');
  for (const input of ['https://chatgpt.com/c/private', 'secret\nprompt', 'a'.repeat(65), 'Secret', 'cookie=value']) {
    assert.equal(failureDetail(input, 'private url'), 'worker_error');
  }
  assert.equal(failureDetail('a'.repeat(64), 'b'.repeat(40)).length, 105);
  assert.equal(failureDetail('ok', 'b'.repeat(41)), 'ok');
});

test('capacity cooldown is capped, durable-time based and expires', () => {
  assert.equal(cooldownRemaining(901000, 1000), 900);
  assert.equal(cooldownRemaining(1001, 1000), 1);
  assert.equal(cooldownRemaining(null, 1000), 0);
  assert.equal(cooldownRemaining(1000, 1001), 0);
  assert.equal(cooldownRemaining(Infinity, 1), 86400);
});

test('UI error classification returns fixed content or capacity codes only', () => {
  for (const text of ['Something went wrong', 'Network error', '发生错误']) assert.equal(classifyChatGptError(text), 'chatgpt_error_response');
  for (const text of ["You've reached the limit for Pro", 'Usage limit reached', '已达到使用上限']) assert.equal(classifyChatGptError(text), 'usage_limit_reached');
  assert.equal(classifyChatGptError('This model is unavailable'), 'model_unavailable');
  assert.equal(classifyChatGptError('Regenerate'), null);
  assert.equal(classifyChatGptError(''), null);
});

test('header target selection verifies family and tier, preferring exact known entries', () => {
  assert.equal(switcherMetadata('GPT-6 Pro'), 'gpt_6_pro');
  assert.equal(switcherMetadata('GPT-6 专业'), 'gpt_6_pro');
  assert.equal(modelLevelTargets('专业')[0], 'Pro');
  assert.equal(switcherMatches('GPT-6 专业', '专业'), true);
  assert.equal(switcherMetadata('GPT-5 Pro'), 'gpt_5_pro');
  assert.equal(switcherMetadata('GPT-6'), 'gpt_6');
  for (const label of ['Try GPT-6 Pro', 'Profile', 'private label']) assert.equal(switcherMetadata(label), 'unrecognized');
  assert.equal(switcherMetadata(null), 'absent');
  assert.equal(switcherMatches('GPT-5 Pro', 'chatgpt-6-pro'), false);
  assert.equal(switcherMatches('GPT-6', 'chatgpt-6-pro'), false);
  assert.equal(switcherMatches('GPT-6 Pro', 'chatgpt-6-pro'), true);
  assert.equal(switcherMatches('unknown', 'unknown'), false);
  assert.equal(chooseSwitcherEntry([{text: 'GPT-6 Pro details'}, {text: 'GPT-6 Pro'}, {text: 'Delete'}], 'chatgpt-6-pro'), 1);
  assert.equal(chooseSwitcherEntry([{text: 'GPT-5 Pro'}, {text: 'Delete'}], 'chatgpt-6-pro'), -1);
  assert.equal(chooseSwitcherEntry([{text: 'ChatGPT-6 Pro details'}, {text: 'ChatGPT-6 Pro'}], 'chatgpt-6-pro'), 1);
});

test('compact numeric labels require adaptation for model matching and never imply effort', () => {
  for (const label of ['6\nPro', '5.5 Pro', 'Pro', '6\nPro\nFor complex work']) {
    assert.equal(switcherMetadata(label), 'unrecognized');
    assert.equal(switcherMatches(label, 'chatgpt-6-pro'), false);
  }
  assert.equal(effortMetadata('6\nPro'), 'unrecognized');
  assert.equal(effortMetadata('GPT 6 Pro'), 'pro');
  assert.equal(chooseSwitcherEntry([{ text: '6\nPro' }], 'chatgpt-6-pro'), 0);
  for (const label of ['6\nPro', '6\n专业', '6\nThinking', '6\n思考']) {
    assert.equal(effortMetadata(label), 'unrecognized', label);
  }
});

test('compact model labels accept only whole numeric family and known tier labels', () => {
  for (const tier of ['Pro', '专业', 'Auto', 'Instant', 'Thinking', 'Medium', 'High', 'Extra High', '自动', '极速', '思考', '均衡', '高级', '超高']) {
    const label = `6\n${tier}`;
    const adapted = compactModelLabel(label);
    assert.equal(adapted, `GPT 6 ${tier}`);
    assert.equal(switcherMetadata(adapted), ['Pro', '专业'].includes(tier) ? 'gpt_6_pro' : 'gpt_6');
    assert.equal(effortMetadata(label), 'unrecognized');
  }
  assert.equal(compactModelLabel(' 5.5\n pRo '), 'GPT 5.5 pRo');
  assert.equal(compactModelLabel('6\nExtra  High'), 'GPT 6 Extra High');
  assert.equal(compactModelLabel('999.999 Pro'), 'GPT 999.999 Pro');
  for (const label of [null, '', 'Pro', '专业', '6', '1000 Pro', '6.1000 Pro', '6.1.2 Pro', '6_1 Pro', '6Pro',
    '6\nPro\nFor complex work', '6 Pro plan', 'Try 6 Pro', '6\nPro Extended', '6\nPro Standard', '6\n扩展', '6\nTools']) {
    assert.equal(compactModelLabel(label), null, label);
    assert.equal(chooseSwitcherEntry([{ text: label }], 'chatgpt-6-pro'), -1, label);
  }
});

test('compact picker entries share family and tier matching with exact entries preferred', () => {
  const items = labels => labels.map(text => ({ text }));
  assert.equal(chooseSwitcherEntry(items(['5.5\nPro', '6\nPro']), 'chatgpt-6-pro'), 1);
  assert.equal(chooseSwitcherEntry(items(['6\nThinking', '6\n专业']), 'chatgpt-6-pro'), 1);
  assert.equal(chooseSwitcherEntry(items(['6\nPro', '6\nThinking']), 'chatgpt-6-thinking'), 1);
  assert.equal(chooseSwitcherEntry(items(['6.2\nPro', '6.1\nPro']), 'chatgpt-6.1-pro'), 1);
  assert.equal(chooseSwitcherEntry(items(['6.1\nPro', '6\nPro']), 'chatgpt-6-pro'), 1);
  assert.equal(chooseSwitcherEntry(items(['GPT-6 Pro\nFor complex work', '6\nPro']), 'chatgpt-6-pro'), 1);
  assert.equal(chooseSwitcherEntry(items(['6.1\nPro', 'GPT-6 Pro']), 'chatgpt-6-pro'), 1);
  assert.equal(chooseSwitcherEntry(items(['60\nPro', '6\nInstant']), 'chatgpt-6-pro'), -1);
  assert.equal(chooseSwitcherEntry(items(['Pro']), 'chatgpt-6-pro', compactModelLabel('6\nThinking')), 0);
  assert.equal(chooseSwitcherEntry(items(['专业']), 'chatgpt-6-pro', compactModelLabel('6\n思考')), 0);
  assert.equal(chooseSwitcherEntry(items(['Pro']), 'chatgpt-6-pro', compactModelLabel('5.5\nThinking')), -1);
  assert.equal(chooseSwitcherFamilyEntry(items(['6', '6\nPro', '6\nThinking']), 'chatgpt-6-pro'), -1);
});

test('effort observations are canonical and Standard is explicit', () => {
  assert.equal(effortMetadata('GPT-6 Pro Extended'), 'pro_extended');
  assert.equal(effortMetadata('Pro 扩展'), 'pro_extended');
  assert.equal(effortMetadata('GPT-6 Pro Standard'), 'pro_standard');
  assert.equal(effortMetadata('private label'), 'unrecognized');
  assert.equal(effortMetadata('Extra High'), 'extra_high');
  assert.equal(effortMetadata(null), 'absent');
  const standard = modelLevelTargets('chatgpt-6-pro-standard');
  assert.equal(chooseNestedLevelEntry([{text:'Pro Extended'}, {text:'Pro Standard'}], standard), 1);
  assert.equal(pillShowsLevel('Pro Extended', standard), false);
  assert.equal(pillShowsLevel('Pro Standard', standard), true);
});

test('an authenticated DOM-shape failure permits exactly one relaunch cycle', () => {
  assert.deepEqual(taskRecoveryDecision({kind:'prompt', phase:'page_ready', failureCount:1, shapeFailures:1}), {action:'recover', forceRelaunch:true});
  assert.deepEqual(taskRecoveryDecision({kind:'prompt', phase:'page_ready', failureCount:2, shapeFailures:2}), {action:'fail', code:'browser_recovery_exhausted'});
  assert.equal(taskRecoveryDecision({kind:'prompt', phase:'send_attempted', failureCount:2, shapeFailures:2}).action, 'recover');
});

test('descriptions and account actions cannot masquerade as the requested Pro tier', () => {
  for (const text of ['GPT-6 Instant\nUpgrade to Pro', 'GPT-6 Instant with Pro features']) {
    assert.equal(switcherMatches(text, 'chatgpt-6-pro'), false);
  }
  for (const text of ['Upgrade to Pro', 'Pro plan', 'GPT-6 Instant\nPro capabilities']) {
    assert.notEqual(detectPillLevel(text), 'Pro');
    assert.equal(pillShowsLevel(text, modelLevelTargets('chatgpt-6-pro')), false);
  }
  assert.equal(switcherMatches('GPT-6 Pro', 'Pro 扩展'), true);
  assert.equal(switcherMatches('GPT-5 Pro', 'Pro 扩展'), false);
  assert.equal(switcherMatches('GPT-6 Pro', '扩展'), true);
});

test('split Pro preference ignores descriptions and refuses the lighter-only fallback', () => {
  const target = modelLevelTargets('chatgpt-6-pro');
  assert.equal(chooseNestedLevelEntry([{text:'Pro Standard'}, {text:'Pro Extended\nFor complex work'}, {text:'Pro'}], target), 1);
  assert.equal(chooseNestedLevelEntry([{text:'Pro Standard', checked:true}], target), -1);
});


test('strict effort decisions require recognized evidence', () => {
  const target = 'chatgpt-6-pro';
  for (const observed of [null, 'Tools', '+', 'Pro']) {
    assert.equal(effortSelectionMismatch({observed, verified:false}, target), false);
  }
  assert.equal(effortSelectionMismatch({observed:'High', verified:false}, target), true);
  assert.equal(effortSelectionMismatch({observed:'Tools', verified:false, recognizedObservation:true}, target), true);
  assert.equal(effortSelectionMismatch({observed:'Pro', verified:false, recognizedObservation:true}, target), false);
  assert.equal(effortSelectionMismatch({observed:'Tools', verified:false, recognizedLevels:true}, target), true);
  assert.equal(effortSelectionMismatch({observed:'Pro Extended', verified:true, recognizedLevels:true}, target), false);
});

test('generic GPT families compare a minor version only when both sides expose one', () => {
  for (const [label, metadata] of [['GPT-6.1 Pro','gpt_6_1_pro'], ['ChatGPT 7 Pro','gpt_7_pro'], ['chatgpt-5.5-pro','gpt_5_5_pro']]) {
    assert.equal(switcherMetadata(label), metadata);
    assert.equal(switcherMatches(label, label), true);
  }
  assert.equal(switcherMatches('GPT-6.1 Pro','chatgpt-6-pro'), true);
  assert.equal(switcherMatches('GPT-6 Pro','chatgpt-6.1-pro'), true);
  assert.equal(switcherMatches('GPT-6.2 Pro','chatgpt-6.1-pro'), false);
  assert.equal(switcherMatches('GPT-60 Pro','chatgpt-6-pro'), false);
  assert.equal(switcherMatches('Try GPT-6 Pro','chatgpt-6-pro'), false);
  assert.equal(switcherMatches('GPT-6 Instant','chatgpt-6-pro', true), true);
  assert.equal(switcherMetadata('GPT-1000 Pro'), 'unrecognized');
});

test('tier-only and family submenu entries require recognized family context and exact first lines', () => {
  const items = ['Auto', 'Instant', 'Thinking', 'Pro\nFor complex work'].map(text => ({text}));
  assert.equal(chooseSwitcherEntry(items, 'chatgpt-6-pro', 'GPT-6'), 3);
  assert.equal(chooseSwitcherEntry(items, 'chatgpt-6-pro', 'GPT-5 Pro'), -1);
  assert.equal(chooseSwitcherEntry([{text:'专业'}], 'chatgpt-6-pro', 'GPT-6'), 0);
  assert.equal(chooseSwitcherEntry([{text:'Upgrade to Pro'}], 'chatgpt-6-pro', 'GPT-6 Instant'), -1);
  const families = ['Legacy models','Try GPT-6','GPT-60','GPT-6\nMore choices'].map(text => ({text}));
  assert.equal(chooseSwitcherFamilyEntry(families, 'chatgpt-6-pro'), 3);
  assert.equal(chooseSwitcherFamilyEntry([{text:'GPT-6 Pro'}], 'chatgpt-6-pro'), -1);
});

test('effort tokens recognize separators without accepting prose', () => {
  for (const label of ['Pro · Extended', 'Thinking: Pro', 'Pro (Extended)', '专业 · 扩展', 'Pro — Extended', 'Pro|Extended', 'Pro/Extended', 'Pro-Extended']) {
    assert.equal(detectPillLevel(label), 'Pro', label);
    assert.equal(effortMetadata(label), label === 'Thinking: Pro' ? 'pro' : 'pro_extended', label);
    assert.equal(chooseNestedLevelEntry([{text:'Pro Standard'}, {text:label}], modelLevelTargets('chatgpt-6-pro')), 1, label);
    if (label !== 'Thinking: Pro') assert.equal(chooseNestedLevelEntry([{text:'Pro'}, {text:label}], modelLevelTargets('chatgpt-6-pro')), 1, label);
  }
  for (const label of ['Upgrade to Pro', 'Pro capabilities', 'Pro plan', 'Profile', 'propro']) {
    assert.equal(detectPillLevel(label), null, label);
    assert.equal(effortMetadata(label), 'unrecognized', label);
  }
});

test('cooldown parsing defaults explicitly for invalid and below-minimum values', () => {
  assert.deepEqual(usageCooldownConfig(undefined), {milliseconds:900000, invalid:false});
  for (const value of ['0', '-1', '', 'invalid', 'NaN', 'Infinity', '0.5']) {
    assert.deepEqual(usageCooldownConfig(value), {milliseconds:900000, invalid:true});
  }
  assert.deepEqual(usageCooldownConfig('1'), {milliseconds:1000, invalid:false});
  assert.deepEqual(usageCooldownConfig('60'), {milliseconds:60000, invalid:false});
  assert.deepEqual(usageCooldownConfig('100000'), {milliseconds:86400000, invalid:false});
});


test('pre-send read-back retries transient errors at most three times', async () => {
  let attempts = 0;
  const observed = {header: {metadata:'gpt_6_pro'}, pill: {observed:'Pro'}};
  const result = await retryPresendModelRead(async () => {
    if (++attempts < 3) throw Object.assign(new Error('read deadline'), {code:'interaction_deadline'});
    return observed;
  });
  assert.equal(result, observed);
  assert.equal(attempts, 3);
  attempts = 0;
  assert.equal(await retryPresendModelRead(async () => { attempts += 1; throw new TypeError('unreadable'); }), null);
  assert.equal(attempts, 3);
});

test('pre-send read-back rethrows crashes and disconnects without retrying', async () => {
  for (const code of ['page_crashed', 'cdp_disconnected']) {
    let attempts = 0;
    const error = Object.assign(new Error(code), {code});
    await assert.rejects(retryPresendModelRead(async () => { attempts += 1; throw error; }), e => e === error);
    assert.equal(attempts, 1);
  }
});

test('pre-send read-back shares PRE_SEND_ACTION_MS across retries and rejects late results', async t => {
  t.mock.timers.enable({apis:['Date', 'setTimeout'], now:10000});
  let attempts = 0;
  let settled = false;
  const budgets = [];
  const pending = retryPresendModelRead(budget => {
    budgets.push(budget);
    const attempt = ++attempts;
    return new Promise((resolve, reject) => setTimeout(() => {
      if (attempt < 3) reject(Object.assign(new Error('read deadline'), {code:'interaction_deadline'}));
      else resolve('late observations');
    }, 1800));
  });
  pending.then(() => { settled = true; });
  const flush = () => new Promise(resolve => setImmediate(resolve));
  for (let i = 0; i < 2; i += 1) {
    t.mock.timers.tick(1800);
    await flush();
  }
  assert.equal(attempts, 3);
  assert.ok(budgets.every(b => b === budgets[0] && b.deadline === 10000 + PRE_SEND_ACTION_MS));
  t.mock.timers.tick(PRE_SEND_ACTION_MS - 3600 - 1);
  await flush();
  assert.equal(settled, false);
  t.mock.timers.tick(1);
  assert.equal(await pending, null);
  assert.equal(Date.now(), 10000 + PRE_SEND_ACTION_MS);
  assert.equal(budgets[0].controller.signal.aborted, true);
  t.mock.timers.tick(PRE_SEND_ACTION_MS);
  await flush();
  assert.equal(attempts, 3);
});


test("composerVisibleHitPoint clamps a tall scrolled composer to its visible centre", () => {
  // Live capture 2026-09-16 on the Heca worker: a 111,353-char draft made the
  // composer 55,864px tall with its top at -55,316px in a 764px viewport, so
  // the geometric centre sat ~27,000px above the viewport. elementFromPoint at
  // that point returned null and the composer was wrongly judged obstructed
  // (composer_unobstructed_failed@selecting_model).
  const rect = { left: 0, right: 800, top: -55316, bottom: 548 };
  const viewport = { width: 1000, height: 764 };
  const geometricCentreY = rect.top + (rect.bottom - rect.top) / 2;
  assert.ok(geometricCentreY < 0); // the old hit-test fell off screen
  const point = composerVisibleHitPoint(rect, [], viewport);
  assert.ok(point);
  assert.ok(point.y >= 0 && point.y <= viewport.height); // the new one does not
  assert.deepEqual(point, { x: 400, y: 274 });
});

test("composerVisibleHitPoint leaves a fully visible composer untouched", () => {
  const rect = { left: 100, right: 900, top: 600, bottom: 700 };
  assert.deepEqual(composerVisibleHitPoint(rect, [], { width: 1000, height: 764 }), { x: 500, y: 650 });
});

test("composerVisibleHitPoint intersects scroll/clip ancestor bounds", () => {
  // Composer spans y 0..800 but a scroll container only reveals 100..500.
  const clip = { y: true, top: 100, bottom: 500, left: 0, right: 1000 };
  assert.deepEqual(
    composerVisibleHitPoint({ left: 0, right: 400, top: 0, bottom: 800 }, [clip], { width: 1000, height: 764 }),
    { x: 200, y: 300 },
  );
});

test("composerVisibleHitPoint returns null when nothing is visible", () => {
  // Entirely below the fold.
  assert.equal(composerVisibleHitPoint({ left: 0, right: 800, top: 2000, bottom: 2100 }, [], { width: 1000, height: 764 }), null);
  // A clipping ancestor hides it on the Y axis.
  const clip = { y: true, top: 0, bottom: 100, left: 0, right: 1000 };
  assert.equal(composerVisibleHitPoint({ left: 0, right: 800, top: 300, bottom: 500 }, [clip], { width: 1000, height: 764 }), null);
  // No composer at all.
  assert.equal(composerVisibleHitPoint(null, [], { width: 1000, height: 764 }), null);
});


test("composerHasDraft treats only non-whitespace content as a draft to clear", () => {
  // A stale draft left in the composer by a prior attempt must be cleared
  // before model selection, or a very long draft makes selection time out
  // (operation_timeout@selecting_model) on every subsequent task pickup.
  assert.equal(composerHasDraft("Review this PR"), true);
  assert.equal(composerHasDraft("6\nPro"), true);
  // An empty or whitespace-only composer is a no-op: nothing to clear.
  assert.equal(composerHasDraft(""), false);
  assert.equal(composerHasDraft("   \n\t "), false);
  // Non-string reads (missing composer) are not drafts.
  assert.equal(composerHasDraft(null), false);
  assert.equal(composerHasDraft(undefined), false);
});
