import assert from "node:assert/strict";
import test from "node:test";
import { readFileSync } from "node:fs";
import { getStateFromPath } from "@react-navigation/core";
import { navigationLinkingConfig } from "./navigationLinking";
import { workspaceStateFromPath } from "./workspaceCanonicalLinks";
const manifest = JSON.parse(
  readFileSync(
    new URL("../../../shared/native-app-links.json", import.meta.url),
    "utf8",
  ),
) as { paths: string[] };
const samples: Record<string, string> = {
  "/assistant/machines/*": "/assistant/machines/node/desktop",
  "/chat/*": "/chat/thread",
  "/groups/*": "/groups/crew",
  "/automations/*": "/automations/task",
  "/agents/*": "/agents/agent",
  "/account/*": "/account/security",
};
test("every claimed AASA/App Link path resolves to a native route", () => {
  for (const claimed of manifest.paths) {
    const path = samples[claimed] ?? claimed;
    assert.ok(
      workspaceStateFromPath(path, (next) =>
        getStateFromPath(next, navigationLinkingConfig),
      ),
      `Claimed native path ${claimed} (${path}) has no native route`,
    );
  }
});
