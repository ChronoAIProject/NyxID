import assert from "node:assert/strict";
import { createRequire } from "node:module";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { getDefaultConfig } from "expo/metro-config";
import type { transform as transformModule } from "@expo/metro-config/build/transform-worker/transform-worker";

const projectRoot = fileURLToPath(new URL("../../", import.meta.url));
const config = getDefaultConfig(projectRoot);
const require = createRequire(import.meta.url);
const { transform } = require(config.transformerPath) as {
  transform: typeof transformModule;
};

async function transformFixture(source: string, platform: "ios" | "android", dev: boolean) {
  // In-memory source still uses the app's Babel config and Expo's Metro worker.
  return transform(config.transformer, projectRoot, "src/lib/LucideFixture.tsx", Buffer.from(source), {
    type: "module",
    platform,
    dev,
    experimentalImportSupport: true,
    inlinePlatform: true,
    inlineRequires: false,
    minify: false,
    unstable_transformProfile: "default",
  });
}

for (const platform of ["ios", "android"] as const) {
  for (const dev of [true, false]) {
    test(`Metro retains the Lucide import condition on ${platform} (dev=${dev})`, async () => {
      const { dependencies } = await transformFixture(`
        import { Heart, HeartIcon as HeartAlias, LucideProvider, useLucideContext as useContext } from "lucide-react-native";
        export const mixed = [Heart, HeartAlias, LucideProvider, useContext];
      `, platform, dev);

      const publicImports = dependencies.filter(({ name }) => name === "lucide-react-native");
      assert.equal(publicImports.length, 1);
      assert.equal(publicImports[0]?.data.isESMImport, true);
      const leaves = dependencies.filter(({ name }) => name.includes("/lucide-react-native/dist/"));
      assert.equal(leaves.length, 1);
      assert.match(leaves[0]!.name, /\/dist\/esm\/icons\/heart\.mjs$/);
    });
  }
}

test("Metro emits no dependencies for Lucide type-only imports alongside a value import", async () => {
  const { dependencies } = await transformFixture(`
    import type { LucideProps } from "lucide-react-native";
    import { Heart, type Circle as CircleType } from "lucide-react-native";
    export const icon = Heart;
    export type IconWithProps = typeof CircleType & LucideProps;
  `, "ios", false);

  const lucide = dependencies.filter(({ name }) => name.includes("lucide-react-native"));
  assert.equal(lucide.length, 1);
  assert.match(lucide[0]!.name, /\/dist\/esm\/icons\/heart\.mjs$/);
});
