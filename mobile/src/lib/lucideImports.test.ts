import assert from "node:assert/strict";
import path from "node:path";
import test from "node:test";
import { transformSync, types as t } from "@babel/core";

const fixture = `
  import { type LucideIcon, Languages, AlertTriangle as Warning, TriangleAlert } from "lucide-react-native";
  export const icons: LucideIcon[] = [Languages, Warning, TriangleAlert];
`;

function imports(
  source: string,
  platform: string,
  metadata: { isNodeModule?: boolean } = { isNodeModule: false },
) {
  const caller = {
    name: "metro",
    platform,
    supportsStaticESM: true,
    ...metadata,
  };
  const filename = path.resolve("src/features/auth/IconFixture.tsx");
  const ast = transformSync(source, {
    filename,
    caller,
    ast: true,
    code: false,
  })?.ast;
  assert.ok(ast);
  return ast.program.body.filter((node) => t.isImportDeclaration(node));
}

test("native Metro imports each icon from its published implementation and erases types", () => {
  for (const platform of ["android", "ios"]) {
    const bindings = imports(fixture, platform).map(
      ({ source, specifiers }) => {
        const [binding] = specifiers;
        assert.ok(t.isImportDefaultSpecifier(binding));
        assert.ok(source.value.startsWith("."));
        return [binding.local.name, path.basename(source.value)];
      },
    );
    assert.deepEqual(bindings, [
      ["Languages", "languages.mjs"],
      ["Warning", "triangle-alert.mjs"],
      ["TriangleAlert", "triangle-alert.mjs"],
    ]);
  }
});

test("jest-expo, third-party modules and web keep Lucide's public entry point", () => {
  for (const declarations of [
    imports(fixture, "ios", {}),
    imports(fixture, "ios", { isNodeModule: true }),
    imports(fixture, "web"),
  ]) {
    assert.deepEqual(
      declarations.map(({ source }) => source.value),
      ["lucide-react-native"],
    );
  }
});

test("mixed imports preserve Lucide's non-icon exports", () => {
  const declarations = imports(
    `
    import { Heart, LucideProvider } from "lucide-react-native";
    export { Heart, LucideProvider };
  `,
    "ios",
  );
  assert.equal(declarations.length, 2);
  assert.ok(
    declarations.some(({ source }) =>
      source.value.endsWith("/icons/heart.mjs"),
    ),
  );
  const root = declarations.find(
    ({ source }) => source.value === "lucide-react-native",
  );
  assert.deepEqual(
    root?.specifiers.map(({ local }) => local.name),
    ["LucideProvider"],
  );
});
