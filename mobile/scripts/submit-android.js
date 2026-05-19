#!/usr/bin/env node
/**
 * Upload an .aab to Google Play Console → Internal testing track,
 * via the Play Developer API (googleapis).
 *
 * Required file:
 *   mobile/credentials/play-service-account.json
 *   (service account with "Release manager" role + app-level access)
 *
 * Required env (resolved through DEV↔PROD fallback):
 *   *_ANDROID_PACKAGE       e.g. fun.chronoai.nyxid
 *
 * Usage:
 *   pnpm submit:android            # auto-picks the .aab at android/.../app-release.aab
 *   pnpm submit:android <path>     # uploads the specified .aab
 */
const fs = require("fs");
const path = require("path");

const { loadEnv, resolveProfile, MOBILE_ROOT } = require("./lib/load-env");

const APP_ENV = process.env.APP_ENV === "dev" ? "dev" : "prod";
const env = loadEnv();
const r = resolveProfile(APP_ENV, env);

const saPath = path.join(MOBILE_ROOT, "credentials", "play-service-account.json");
if (!fs.existsSync(saPath)) {
  console.error(
    `[submit-android] Service account JSON not found at: ${saPath}\n` +
      "Create one in Play Console → Setup → API access, grant it 'Release manager' role,\n" +
      "and download the JSON key.",
  );
  process.exit(1);
}

let aabPath = process.argv[2];
if (!aabPath) {
  const candidate = path.join(
    MOBILE_ROOT,
    "android",
    "app",
    "build",
    "outputs",
    "bundle",
    "release",
    "app-release.aab",
  );
  if (!fs.existsSync(candidate)) {
    console.error(
      "[submit-android] No .aab specified and default path not found:\n" +
        `  ${candidate}\nRun pnpm build:android first.`,
    );
    process.exit(1);
  }
  aabPath = candidate;
}
if (!fs.existsSync(aabPath)) {
  console.error(`[submit-android] .aab not found: ${aabPath}`);
  process.exit(1);
}

const pkg = r.androidPackage;
const track = "internal";

console.log(`[submit-android] Package: ${pkg}`);
console.log(`[submit-android] Track:   ${track}`);
console.log(`[submit-android] Uploading: ${aabPath}\n`);

async function main() {
  let google;
  try {
    google = require("googleapis").google;
  } catch (e) {
    if (e && e.code === "MODULE_NOT_FOUND") {
      console.error("[submit-android] `googleapis` is not installed. Run `pnpm install` in mobile/.");
      process.exit(1);
    }
    throw e;
  }

  const auth = new google.auth.GoogleAuth({
    keyFile: saPath,
    scopes: ["https://www.googleapis.com/auth/androidpublisher"],
  });
  const play = google.androidpublisher({ version: "v3", auth });

  console.log("[submit-android] 1/5: edits.insert");
  const insertRes = await play.edits.insert({ packageName: pkg });
  const editId = insertRes.data.id;
  if (!editId) throw new Error("edits.insert returned no id");

  console.log(`[submit-android] 2/5: edits.bundles.upload (editId=${editId})`);
  const uploadRes = await play.edits.bundles.upload({
    packageName: pkg,
    editId,
    media: { mimeType: "application/octet-stream", body: fs.createReadStream(aabPath) },
  });
  const versionCode = uploadRes.data.versionCode;
  console.log(`[submit-android]    uploaded versionCode=${versionCode}`);

  console.log("[submit-android] 3/5: edits.tracks.update → internal");
  await play.edits.tracks.update({
    packageName: pkg,
    editId,
    track,
    requestBody: {
      track,
      releases: [
        {
          status: "completed",
          versionCodes: [String(versionCode)],
        },
      ],
    },
  });

  console.log("[submit-android] 4/5: edits.commit");
  await play.edits.commit({ packageName: pkg, editId });

  console.log("[submit-android] 5/5: ✓ uploaded — Play Console → Testing → Internal testing");
}

main().catch((e) => {
  console.error("\n[submit-android] failed:");
  if (e && e.errors) {
    console.error(JSON.stringify(e.errors, null, 2));
  } else if (e && e.message) {
    console.error(e.message);
  } else {
    console.error(e);
  }
  console.error("\nCommon causes:");
  console.error("  - versionCode ≤ last accepted (bump *_ANDROID_VERSION_CODE and rebuild)");
  console.error("  - Service account missing 'Release manager' permission on this app");
  console.error("  - Package name mismatch with the Play Console app");
  process.exit(1);
});
