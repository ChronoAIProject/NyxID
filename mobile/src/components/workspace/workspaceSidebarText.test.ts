import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { sidebarTextProps } from "./sidebarText";

test("P9 ordinary sidebar labels retain native paragraph identity and the drawer close gesture", () => {
  for (const name of [
    "WorkspaceDrawer",
    "WorkspaceChatList",
    "WorkspaceRecents",
    "SidebarSectionHeader",
  ]) {
    const source = readFileSync(
      new URL(`./${name}.tsx`, import.meta.url),
      "utf8",
    );
    assert.doesNotMatch(
      source,
      /DynamicTypeText as Text/,
      `${name}: keep base paragraph identity`,
    );
    assert.match(source, /import \{[^}]*\bText\b[^}]*\} from "react-native"/);
    assert.doesNotMatch(
      source,
      /SidebarHeaderViewport|<ScrollView[^>]*\bhorizontal/s,
    );
  }
  assert.deepEqual(sidebarTextProps(), {
    numberOfLines: 1,
    ellipsizeMode: "tail",
  });
});
