const { appendFile } = require("node:fs/promises");
const { measureArtifact } = require("./lib/artifact-size");

const profiles = new Map([
  ["android-aab", { label: "Android release AAB", kind: "aab", limitMiB: 110 }],
  [
    "android-arm64-apk",
    { label: "Android ARM64 QA APK", kind: "apk", limitMiB: 70 },
  ],
  ["ios-ipa", { label: "iOS signed IPA", kind: "ipa", limitMiB: 25 }],
  [
    "ios-reference-ipa",
    {
      label: "iOS unsigned device IPA (CI reference)",
      kind: "ipa",
      limitMiB: 30,
    },
  ],
]);

async function reportPackageSize(profile, file) {
  const config = profiles.get(profile);
  if (!config || !file)
    throw new Error("Expected a package profile and a matching file path");
  const size = await measureArtifact(config.kind, file);
  if (!Number.isSafeInteger(size) || size < 1)
    throw new Error("Empty or invalid package");

  const bytesPerMiB = 1024 * 1024;
  const limit = config.limitMiB * bytesPerMiB;
  const over = size > limit;
  const display = (size / bytesPerMiB).toFixed(2) + " MiB";
  const status = over ? "Over budget" : "Within budget";
  const message = `${config.label} • ${display} (${size} bytes) • ${config.limitMiB} MiB budget • ${status}`;
  if (over && process.env.GITHUB_ACTIONS === "true")
    console.warn(`::warning title=Mobile package size::${message}`);
  else console.log(message);

  if (process.env.GITHUB_STEP_SUMMARY) {
    await appendFile(
      process.env.GITHUB_STEP_SUMMARY,
      [
        "### Mobile package size",
        "",
        "| Package | File size | Bytes | Budget | Result |",
        "| --- | ---: | ---: | ---: | --- |",
        `| ${config.label} | ${display} | ${size} | ${config.limitMiB} MiB | ${status} |`,
        "",
        "",
      ].join("\n"),
    );
  }
}

module.exports = { reportPackageSize };

if (require.main === module) {
  reportPackageSize(process.argv[2], process.argv[3]).catch((error) => {
    console.error(`Package size report failed. ${error.code || error.message}`);
    process.exitCode = 1;
  });
}
