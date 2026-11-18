import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import ts from "typescript";
import { sidebarTextProps } from "./sidebarText";

test("every sidebar Text is single-line without competing with the drawer close gesture", () => {
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
    assert.match(
      source,
      /DynamicTypeText as Text/,
      `${name}: live font changes must refresh native paragraph metrics`,
    );
    assert.doesNotMatch(source, /SidebarHeaderViewport|<ScrollView[^>]*\bhorizontal/s);
    const file = ts.createSourceFile(
      name + ".tsx",
      source,
      ts.ScriptTarget.Latest,
      true,
    );
    const visit = (node: ts.Node) => {
      if (
        ts.isJsxOpeningElement(node) &&
        node.tagName.getText(file) === "Text"
      ) {
        const props = node.attributes.properties;
        assert.ok(
          props.some(
            (p) =>
              ts.isJsxAttribute(p) &&
              p.name.getText(file) === "numberOfLines" &&
              p.initializer?.getText(file) === "{1}",
          ),
          `${name}: ${node.getText(file)}`,
        );
        assert.ok(
          props.some(
            (p) =>
              ts.isJsxSpreadAttribute(p) &&
              p.expression.getText(file).startsWith("sidebarTextProps("),
          ),
          `${name}: missing shared label policy`,
        );

      }
      ts.forEachChild(node, visit);
    };
    visit(file);
  }
  assert.deepEqual(sidebarTextProps(), {
    numberOfLines: 1,
    ellipsizeMode: "tail",
  });
});
