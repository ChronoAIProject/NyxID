import assert from "node:assert/strict";
import test from "node:test";
import {
  setAppTranslationLanguage,
  translateApp,
} from "../../i18n/appTranslation";
import {
  QUICK_ACCESS_DEFAULT_STATE,
  QUICK_ACCESS_DESTINATIONS,
  type QuickAccessItem,
} from "../../lib/api/quickAccessSchema";
import {
  addQuickAccess,
  moveQuickAccess,
  quickAccessCandidates,
  quickAccessDestinationLabel,
  quickAccessFull,
  removeQuickAccess,
  resolveQuickAccess,
  type QuickAccessAgent,
  type QuickAccessCrew,
} from "./quickAccessModel";

const AGENT = "6f9619ff-8b86-d011-b42d-00c04fc964ff";
const DESTROYED = "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee";
const GONE = "11111111-2222-4333-8444-555555555555";
const CREW = "nyxg-0123456789abcdef0123456789abcdef";
const agents: QuickAccessAgent[] = [
  { id: AGENT, name: "researcher", display_name: "Research", kind: "specialist", status: "idle" },
  { id: DESTROYED, name: "old", kind: "specialist", status: "destroyed" },
  { id: "22222222-3333-4444-8555-666666666666", name: "nyxbot", display_name: "NyxBot", kind: "nyxbot", status: "running" },
];
const crews: QuickAccessCrew[] = [
  { id: CREW, name: "Launch crew", members: [{ id: AGENT, name: "researcher", kind: "specialist" }] },
];
const destination = (id: (typeof QUICK_ACCESS_DESTINATIONS)[number]): QuickAccessItem => ({
  kind: "destination",
  id,
});

test("every destination has the side menu's translated name", () => {
  assert.deepEqual(
    QUICK_ACCESS_DESTINATIONS.map(quickAccessDestinationLabel),
    [
      "New chat",
      "Notifications",
      "Scan QR code",
      "Terminal login code",
      "Recent chats",
      "Automations",
      "Machines",
      "Plugins",
      "Activity",
      "Create new agent",
      "Settings",
      "Agents",
      "Crews",
    ],
  );
  const english = QUICK_ACCESS_DESTINATIONS.map(quickAccessDestinationLabel);
  try {
    for (const language of ["zh-CN", "zh-TW"] as const) {
      setAppTranslationLanguage(language);
      QUICK_ACCESS_DESTINATIONS.forEach((id, index) => {
        const label = quickAccessDestinationLabel(id);
        assert.ok(label.trim(), `${language}: ${id}`);
        assert.notEqual(label, english[index], `${language}: ${id} is translated`);
      });
      assert.notEqual(
        translateApp("Chat with {{label}}", { label: "X" }),
        "Chat with X",
        language,
      );
    }
  } finally {
    setAppTranslationLanguage("en");
  }
});

test("the rail shows saved entries in order, skipping deleted, destroyed and not-yet-loaded agents and crews", () => {
  const items: QuickAccessItem[] = [
    { kind: "agent", id: AGENT.toUpperCase() },
    destination("machines"),
    { kind: "agent", id: GONE },
    { kind: "agent", id: DESTROYED },
    { kind: "crew", id: CREW },
    { kind: "crew", id: "nyxg-ffffffffffffffffffffffffffffffff" },
    destination("notifications"),
  ];
  const resolved = resolveQuickAccess(items, { agents, crews, notificationsEnabled: true });
  assert.deepEqual(
    resolved.map((entry) => [entry.kind, entry.name, entry.label]),
    [
      ["agent", "Research", "Chat with Research"],
      ["destination", "Machines", "Machines"],
      ["crew", "Launch crew", "Open crew Launch crew"],
      ["destination", "Notifications", "Notifications"],
    ],
  );
  // Each entry carries its saved item (what opens) and a stable key.
  assert.equal(resolved[0]!.item, items[0]);
  assert.equal(new Set(resolved.map((entry) => entry.key)).size, resolved.length);
  // Notifications follows its feature check, like the side menu's row.
  assert.deepEqual(
    resolveQuickAccess(items, { agents, crews, notificationsEnabled: false }).map(
      (entry) => entry.name,
    ),
    ["Research", "Machines", "Launch crew"],
  );
  // Lists not loaded (or failed): pinned agents and crews wait.
  assert.deepEqual(
    resolveQuickAccess(items, { notificationsEnabled: true }).map((entry) => entry.name),
    ["Machines", "Notifications"],
  );
  // Absent from the account: the default five.
  assert.deepEqual(
    resolveQuickAccess(QUICK_ACCESS_DEFAULT_STATE.items, { notificationsEnabled: true }).map(
      (entry) => entry.name,
    ),
    ["New chat", "Notifications", "Scan QR code", "Recent chats", "Settings"],
  );
});

test("add, remove and reorder keep the list valid: no duplicates, at most 12, ends stay put", () => {
  const start = [destination("new_chat"), destination("settings")];
  assert.deepEqual(addQuickAccess(start, destination("plugins")), [...start, destination("plugins")]);
  assert.deepEqual(addQuickAccess(start, destination("settings")), start, "already there");
  assert.deepEqual(
    addQuickAccess([{ kind: "agent", id: AGENT }], { kind: "agent", id: AGENT.toUpperCase() }),
    [{ kind: "agent", id: AGENT }],
  );
  const twelve = QUICK_ACCESS_DESTINATIONS.slice(0, 12).map(destination);
  assert.ok(quickAccessFull(twelve));
  assert.ok(!quickAccessFull(start));
  assert.deepEqual(addQuickAccess(twelve, destination("crews")), twelve, "full");
  assert.deepEqual(removeQuickAccess(start, 0), [destination("settings")]);
  assert.deepEqual(moveQuickAccess(start, 1, -1), [destination("settings"), destination("new_chat")]);
  assert.deepEqual(moveQuickAccess(start, 0, 1), [destination("settings"), destination("new_chat")]);
  assert.deepEqual(moveQuickAccess(start, 0, -1), start);
  assert.deepEqual(moveQuickAccess(start, 1, 1), start);
  // Helpers never mutate the list they were given.
  assert.deepEqual(start, [destination("new_chat"), destination("settings")]);
});

test("candidates are what is not yet in the list: destinations, live agents and crews", () => {
  const items: QuickAccessItem[] = [
    destination("new_chat"),
    { kind: "agent", id: AGENT },
  ];
  const candidates = quickAccessCandidates(items, { agents, crews });
  assert.equal(candidates.destinations.includes("new_chat"), false);
  assert.equal(candidates.destinations.length, QUICK_ACCESS_DESTINATIONS.length - 1);
  // Destroyed agents can't be pinned; the one already pinned is not offered.
  assert.deepEqual(candidates.agents.map((agent) => agent.name), ["nyxbot"]);
  assert.deepEqual(candidates.crews.map((crew) => crew.id), [CREW]);
  assert.deepEqual(quickAccessCandidates(items, {}).agents, []);
});
