import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { runInNewContext } from "node:vm";
import test from "node:test";
import ts from "typescript";
import type { ChallengeDetail } from "../lib/api/types";

type Element = { type: string; props: Record<string, any> };

// Exercise the real card source; replace only native rendering and theme.
function loadCard(name: "HistoryCard" | "ChallengeCard") {
  const filename = fileURLToPath(new URL(`./${name}.tsx`, import.meta.url));
  const output = ts.transpileModule(readFileSync(filename, "utf8"), {
    compilerOptions: {
      module: ts.ModuleKind.CommonJS,
      target: ts.ScriptTarget.ES2022,
      jsx: ts.JsxEmit.ReactJSX,
      esModuleInterop: true,
    },
  }).outputText;
  const mocks: Record<string, unknown> = {
    react: { useMemo: (create: () => unknown) => create() },
    "react/jsx-runtime": {
      jsx: (type: string, props: unknown) => ({ type, props }),
      jsxs: (type: string, props: unknown) => ({ type, props }),
    },
    "react-native": {
      Pressable: "Pressable",
      Text: "Text",
      View: "View",
      StyleSheet: { create: (styles: unknown) => styles },
    },
    "expo-blur": { BlurView: "BlurView" },
    "../theme/designTokens": {
      TOUCH_TARGET: 44,
      radius: {},
      spacing: {},
      typeScale: {},
    },
    "../theme/ThemeContext": {
      useTheme: () => ({ colors: {}, mode: "light" }),
    },
    "./StatusBadge": { StatusBadge: "StatusBadge" },
  };
  const exports: Record<string, (props: any) => Element> = {};
  runInNewContext(
    output,
    {
      exports,
      require: (dependency: string) => {
        assert.ok(dependency in mocks, `Unexpected import: ${dependency}`);
        return mocks[dependency];
      },
      Date,
    },
    { filename },
  );
  return exports[name];
}

function elements(root: Element): Element[] {
  return [
    root,
    ...[root.props.children]
      .flat(Infinity)
      .flatMap((child) =>
        child && typeof child === "object" && "props" in child
          ? elements(child)
          : [],
      ),
  ];
}
function textContent(root: Element): string {
  return [root.props.children]
    .flat(Infinity)
    .map((child) =>
      child && typeof child === "object" && "props" in child
        ? textContent(child)
        : typeof child === "string" || typeof child === "number"
          ? String(child)
          : "",
    )
    .join("");
}
function badgeLabels(root: Element): string[] {
  return elements(root)
    .filter((e) => e.type === "StatusBadge")
    .map((e) => e.props.label);
}
function displayTime(value: string): string {
  return new Date(value).toLocaleString("en-US", {
    month: "short",
    day: "numeric",
    year: "numeric",
    hour: "numeric",
    minute: "2-digit",
  });
}
const request: ChallengeDetail = {
  id: "approval-request",
  title: "Cloudflare",
  service_slug: "cloudflare",
  action: "POST",
  resource: "/zones/zone-id/dns_records",
  summary:
    "Create a DNS record in the production zone for api.example.com, pointing to 192.0.2.1 with proxying enabled.",
  request_context: { client: "agent", requester: "AuricBot · nyx_key_abcd" },
  status: "PENDING",
  approval_mode: "per_request",
  risk_level: "unknown",
  created_at: "2026-10-12T10:00:00Z",
  expires_at: "2099-01-01T00:00:00Z",
};
const HistoryCard = loadCard("HistoryCard");
const ChallengeCard = loadCard("ChallengeCard");

test("pending card shows the complete operation, requester and service, and opens its detail", () => {
  let opened = 0;
  const tree = ChallengeCard({
    challenge: { ...request, from_org_policy: true, org_name: "Chrono" },
    onPress: () => opened++,
  });
  const summary = elements(tree).find(
    (e) => e.type === "Text" && textContent(e) === request.summary,
  );
  assert.ok(summary);
  assert.equal(summary.props.numberOfLines, undefined);
  assert.match(textContent(tree), /Cloudflare/);
  assert.match(textContent(tree), /Requested by AuricBot · nyx_key_abcd/);
  assert.deepEqual(badgeLabels(tree), ["Org · Chrono", "Per-request"]);
  assert.equal(tree.props.accessibilityRole, "button");
  assert.match(tree.props.accessibilityHint, /details/);
  tree.props.onPress();
  assert.equal(opened, 1);
});

test("pending card retains decision callbacks and known risk/grant evidence", () => {
  let approved = 0,
    denied = 0;
  const tree = ChallengeCard({
    challenge: { ...request, risk_level: "high", approval_mode: "grant" },
    grantDurationLabel: "7 days",
    onApprove: () => approved++,
    onDeny: () => denied++,
  });
  assert.deepEqual(badgeLabels(tree), ["High risk", "Grant mode"]);
  assert.match(
    textContent(tree),
    /Approving allows matching requests for 7 days/,
  );
  for (const label of ["Approve", "Deny"]) {
    const button = elements(tree).find(
      (e) => e.type === "Pressable" && textContent(e) === label,
    );
    assert.ok(button);
    assert.equal(button.props.disabled, false);
    button.props.onPress();
  }
  assert.equal(approved, 1);
  assert.equal(denied, 1);
});

test("pending decisions remain blocked for unknown/elapsed expiry, settled status and mutation", () => {
  for (const [challenge, isMutating] of [
    [{ ...request, expires_at: "" }, false],
    [{ ...request, expires_at: "2020-01-01T00:00:00Z" }, false],
    [{ ...request, status: "APPROVED" }, false],
    [request, true],
  ] as const) {
    const tree = ChallengeCard({ challenge, isMutating });
    const buttons = elements(tree).filter(
      (e) =>
        e.type === "Pressable" && ["Approve", "Deny"].includes(textContent(e)),
    );
    assert.equal(buttons.length, 2);
    assert.ok(buttons.every((button) => button.props.disabled));
  }
});

test("history shows actual decision channel and decision time without attributing it to the reader", () => {
  for (const [decision_channel, label] of [
    ["web", "Web"],
    ["telegram", "Telegram"],
    ["mobile", "Mobile"],
    ["push", "Mobile"],
  ]) {
    const decided_at = "2026-10-14T15:30:00Z";
    const tree = HistoryCard({
      item: { ...request, status: "APPROVED", decided_at, decision_channel },
    });
    const text = textContent(tree);
    assert.ok(text.includes(request.summary));
    assert.ok(text.includes(`Cloudflare · via ${label}`));
    assert.ok(text.includes(`Decided ${displayTime(decided_at)}`));
    assert.ok(text.includes("Requested by AuricBot · nyx_key_abcd"));
    assert.ok(!text.includes("You approved"));
    assert.ok(!text.includes(displayTime(request.created_at)));
    assert.deepEqual(badgeLabels(tree), ["Approved"]);
    assert.equal(
      elements(tree).find(
        (e) => e.type === "Text" && textContent(e) === request.summary,
      )?.props.numberOfLines,
      undefined,
    );
  }
});

test("history falls back to request time without inventing a decision timestamp or channel", () => {
  for (const decided_at of [undefined, null, "invalid"]) {
    const tree = HistoryCard({
      item: { ...request, status: "DENIED", decided_at },
    });
    assert.match(textContent(tree), /Requested/);
    assert.ok(textContent(tree).includes(displayTime(request.created_at)));
    assert.doesNotMatch(textContent(tree), /Decided|via |You denied/);
    assert.deepEqual(badgeLabels(tree), ["Denied"]);
  }
});

test("history preserves detail navigation and known risk without truncating the request", () => {
  let opened = 0;
  const tree = HistoryCard({
    item: { ...request, status: "EXPIRED", risk_level: "medium" },
    onPress: () => opened++,
  });
  tree.props.onPress();
  assert.equal(opened, 1);
  assert.deepEqual(badgeLabels(tree), ["Expired", "Medium risk"]);
  assert.ok(textContent(tree).includes(request.summary));
});

test("the request expiry and one-request scope are visible before deciding", () => {
  const expires_at = new Date(Date.now() + 180_000).toISOString();
  const single = ChallengeCard({ challenge: { ...request, expires_at } });
  assert.match(textContent(single), /Expires in 3 min/);
  assert.match(textContent(single), /Approval applies only to this request/);
  const invalid = ChallengeCard({ challenge: { ...request, expires_at: "" } });
  assert.match(textContent(invalid), /Expiry unavailable/);
});

test("offline decisions are disabled while cached request details remain readable", () => {
  const tree = ChallengeCard({
    challenge: request,
    decisionsDisabled: true,
    onPress: () => {},
  });
  assert.equal(tree.props.disabled, false);
  const decisions = elements(tree).filter(
    (e) =>
      e.type === "Pressable" && ["Approve", "Deny"].includes(textContent(e)),
  );
  assert.equal(decisions.length, 2);
  assert.ok(decisions.every((button) => button.props.disabled));
});
