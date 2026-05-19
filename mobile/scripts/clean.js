#!/usr/bin/env node
/**
 * Force-clean build state when the build scripts can't recover on their own.
 *
 * Usage:
 *   pnpm clean:ios          # wipes ios/build/ + ios/Pods/ + ios/build/generated/
 *   pnpm clean:android      # wipes android/ (prebuild does this anyway, but explicit)
 *   pnpm clean:caches       # wipes .expo/ and the project's Xcode DerivedData
 *   pnpm clean              # all of the above (everything but node_modules)
 *   pnpm clean:full         # all of the above + node_modules + pnpm-lock.yaml entries
 *
 * The day-to-day build scripts only remove old artifacts (.ipa, .xcarchive)
 * and let pod install / gradle handle incremental rebuilds. Use clean:* when
 * a build mysteriously fails or you suspect cache corruption.
 */
const fs = require("fs");
const path = require("path");
const os = require("os");

const { MOBILE_ROOT } = require("./lib/load-env");

const target = process.argv[2] || "all";

const PATHS = {
  ios: [
    path.join(MOBILE_ROOT, "ios", "build"),
    path.join(MOBILE_ROOT, "ios", "Pods"),
    path.join(MOBILE_ROOT, "ios", "Podfile.lock"),
  ],
  android: [path.join(MOBILE_ROOT, "android")],
  caches: [
    path.join(MOBILE_ROOT, ".expo"),
    // Xcode DerivedData for this project — folder name starts with "NyxIDMobile-"
    // Compute glob at runtime since the suffix is hash-based.
  ],
  node: [
    path.join(MOBILE_ROOT, "node_modules"),
  ],
};

function deletePath(p) {
  if (!fs.existsSync(p)) return;
  const rel = path.relative(MOBILE_ROOT, p);
  console.log(`  rm ${rel}`);
  fs.rmSync(p, { recursive: true, force: true });
}

function deleteXcodeDerivedData() {
  const ddRoot = path.join(os.homedir(), "Library", "Developer", "Xcode", "DerivedData");
  if (!fs.existsSync(ddRoot)) return;
  for (const entry of fs.readdirSync(ddRoot)) {
    if (entry.startsWith("NyxIDMobile-")) {
      const full = path.join(ddRoot, entry);
      console.log(`  rm ~/Library/Developer/Xcode/DerivedData/${entry}`);
      fs.rmSync(full, { recursive: true, force: true });
    }
  }
}

const sets = {
  ios: () => {
    console.log("[clean] ios:");
    PATHS.ios.forEach(deletePath);
  },
  android: () => {
    console.log("[clean] android:");
    PATHS.android.forEach(deletePath);
  },
  caches: () => {
    console.log("[clean] caches:");
    PATHS.caches.forEach(deletePath);
    deleteXcodeDerivedData();
  },
  node: () => {
    console.log("[clean] node:");
    PATHS.node.forEach(deletePath);
  },
};

if (target === "ios") sets.ios();
else if (target === "android") sets.android();
else if (target === "caches") sets.caches();
else if (target === "all") {
  sets.ios();
  sets.android();
  sets.caches();
} else if (target === "full") {
  sets.ios();
  sets.android();
  sets.caches();
  sets.node();
} else {
  console.error(`Unknown target: ${target}. Use: ios | android | caches | all | full`);
  process.exit(1);
}

console.log("\n[clean] done.");
