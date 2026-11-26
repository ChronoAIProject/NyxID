const { readFileSync } = require("node:fs");
const path = require("node:path");

// Lucide publishes its native ESM entry in the manifest but doesn't export the manifest.
const manifest = require.resolve("./lucide-react-native/package.json", {
  paths: require.resolve.paths("lucide-react-native"),
});
const lucideEntry = path.resolve(
  path.dirname(manifest),
  require(manifest)["react-native"],
);

function lucideIcons({ types: t, parseSync }) {
  // Read Lucide's own re-exports so aliases always resolve to the same component.
  const entry = parseSync(readFileSync(lucideEntry, "utf8"), {
    configFile: false,
    babelrc: false,
    sourceType: "module",
  });
  const icons = new Map();
  for (const declaration of entry.program.body) {
    if (
      !t.isExportNamedDeclaration(declaration) ||
      !declaration.source?.value.startsWith("./icons/")
    ) {
      continue;
    }
    for (const specifier of declaration.specifiers) {
      if (
        t.isExportSpecifier(specifier) &&
        specifier.local.name === "default"
      ) {
        const file = path.resolve(
          path.dirname(lucideEntry),
          declaration.source.value,
        );
        icons.set(specifier.exported.name, file);
      }
    }
  }

  return {
    name: "lucide-icon-imports",
    visitor: {
      ImportDeclaration(importPath, state) {
        const { node } = importPath;
        if (
          node.source.value !== "lucide-react-native" ||
          node.importKind === "type"
        )
          return;

        const leaves = [];
        const remaining = [];
        for (const specifier of node.specifiers) {
          if (
            !t.isImportSpecifier(specifier) ||
            specifier.importKind === "type" ||
            !icons.has(specifier.imported.name)
          ) {
            remaining.push(specifier);
            continue;
          }
          const file = icons.get(specifier.imported.name);
          const relative = path
            .relative(path.dirname(state.filename), file)
            .split(path.sep)
            .join("/");
          leaves.push(
            t.importDeclaration(
              [t.importDefaultSpecifier(specifier.local)],
              t.stringLiteral(
                relative.startsWith(".") ? relative : "./" + relative,
              ),
            ),
          );
        }
        if (!leaves.length) return;
        if (remaining.length)
          leaves.push(t.importDeclaration(remaining, node.source));
        importPath.replaceWithMultiple(leaves);
      },
    },
  };
}

module.exports = { lucideEntry, lucideIcons };
