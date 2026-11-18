import assert from "node:assert/strict";
import test from "node:test";
import {
  historyActor,
  historyChange,
  historySummary,
  historyValue,
  parseConnectionHistory,
  parseHistoryActions,
  type ConnectionHistoryGroup,
} from "./pluginConnectionHistoryTypes";

const event = (
  action: string,
  id = action,
): ConnectionHistoryGroup["events"][number] => ({
  id,
  schema_version: 1,
  action,
  action_label: "Connection updated",
  actor: {
    kind: "person",
    id: "owner",
    name: "Owner",
    person_id: "owner",
    api_key_id: null,
    app_id: null,
  },
  committed_at: "2026-10-14T00:00:00Z",
  additional_changes: false,
  audit_status: "published",
  changes: [],
});
const page = () => ({
  service_id: "connection-a",
  next_cursor: "next-page",
  tracked_since: null,
  legacy: false,
  deleted: false,
  groups: [{ id: "change-group", events: [event("service.updated")] }],
});
const options = (overrides: Record<string, unknown> = {}) => ({
  option_set: "service-history-action",
  items: [
    { value: "service.created", label: "Service created", disabled: false },
  ],
  total: 11,
  next_offset: null,
  version: "definitions-1",
  ...overrides,
});

test("history preserves real grouped events and terminal pagination", () => {
  const first = parseConnectionHistory(page(), "connection-a", null);
  assert.equal(first.groups[0]?.events[0]?.action, "service.updated");
  const last = parseConnectionHistory(
    { ...page(), next_cursor: null },
    "connection-a",
    first.next_cursor,
  );
  assert.equal(last.next_cursor, null);
});

test("history rejects another connection and a non-advancing cursor", () => {
  assert.throws(
    () => parseConnectionHistory(page(), "connection-b", null),
    /different connection/,
  );
  assert.throws(
    () => parseConnectionHistory(page(), "connection-a", "next-page"),
    /invalid cursor/,
  );
});

test("deleted and legacy histories remain readable without a live connection", () => {
  const result = parseConnectionHistory(
    {
      ...page(),
      deleted: true,
      legacy: true,
      tracked_since: "2026-10-01T00:00:00Z",
    },
    "connection-a",
    null,
  );
  assert.equal(result.deleted, true);
  assert.equal(result.legacy, true);
  assert.equal(result.groups.length, 1);
});

test("invalid audit state and incomplete authorship cannot be displayed as valid history", () => {
  const malformed = page();
  assert.throws(() =>
    parseConnectionHistory(
      {
        ...malformed,
        groups: [
          {
            id: "g",
            events: [{ ...event("service.updated"), audit_status: "unknown" }],
          },
        ],
      },
      "connection-a",
      null,
    ),
  );
  assert.throws(() =>
    parseConnectionHistory(
      {
        ...malformed,
        groups: [
          {
            id: "g",
            events: [
              {
                ...event("service.updated"),
                actor: { name: "Missing identity" },
              },
            ],
          },
        ],
      },
      "connection-a",
      null,
    ),
  );
});

test("dynamic history options reject wrong context, pagination loops, and changed versions", () => {
  assert.equal(
    parseHistoryActions(options(), 0).items[0]?.label,
    "Service created",
  );
  assert.equal(
    parseHistoryActions(options({ next_offset: 5 }), 0).next_offset,
    5,
  );
  assert.throws(() =>
    parseHistoryActions(options({ option_set: "service-scope" }), 0),
  );
  assert.throws(
    () => parseHistoryActions(options({ next_offset: 5 }), 5),
    /invalid pagination/,
  );
  assert.throws(
    () => parseHistoryActions(options({ next_offset: 11 }), 0),
    /invalid pagination/,
  );
  assert.throws(
    () => parseHistoryActions(options(), 0, "older-definitions"),
    /filters changed/,
  );
});

test("summary follows Web deletion, creation, then latest-event priority", () => {
  const updated = event("service.updated");
  const created = event("service.created");
  const deleted = event("service.deleted");
  assert.equal(
    historySummary({ id: "g", events: [created, deleted, updated] }),
    deleted,
  );
  assert.equal(
    historySummary({ id: "g", events: [created, updated] }),
    created,
  );
  assert.equal(historySummary({ id: "g", events: [updated] }), updated);
  assert.equal(historySummary({ id: "g", events: [] }), undefined);
});

test("metadata formatting exposes only supported structured summaries", () => {
  assert.equal(
    historyValue("token_scopes", ["profile:read", "services:list"]),
    "profile:read, services:list",
  );
  assert.equal(historyValue("token_scopes", []), "No reviewed scopes");
  assert.equal(
    historyValue("default_request_headers", {
      count: 2,
      names: ["X-Workspace", "X-Region"],
      arbitrary: "hidden",
    }),
    "2 headers (X-Workspace, X-Region)",
  );
  assert.equal(
    historyValue("ws_frame_injections", {
      count: 1,
      directions: ["downstream"],
      payload: "hidden",
    }),
    "1 rule (downstream)",
  );
  assert.equal(
    historyValue("unknown", { credential: "never serialize objects" }),
    "Changed",
  );
  assert.equal(
    historyValue("token_scopes", ["read", { secret: "hidden" }]),
    "Changed",
  );
});

test("change details preserve omitted-value and primitive semantics", () => {
  assert.equal(
    historyChange({ field: "credential", before: null, after: null }),
    "Changed; values omitted",
  );
  assert.equal(
    historyChange({ field: "enabled", before: false, after: true }),
    "No → Yes",
  );
  assert.equal(
    historyChange({ field: "label", before: null, after: "Personal account" }),
    "Not recorded → Personal account",
  );
});

test("actor labels match Web and ignore inherited object keys", () => {
  const actor = event("service.updated").actor;
  assert.equal(historyActor({ ...actor, kind: "api_key" }), "Owner (API key)");
  assert.equal(historyActor({ ...actor, kind: "system" }), "Owner (System)");
  assert.equal(historyActor({ ...actor, kind: "constructor" }), "Owner");
});
