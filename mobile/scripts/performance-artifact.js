const fs = require("node:fs/promises");
const path = require("node:path");
const { createHash } = require("node:crypto");
const { parseArgs } = require("node:util");
const { loadEnv } = require("./lib/load-env");

const artifacts = new Map([
  ["aab", ["android", ".aab"]],
  ["apk", ["android", ".apk"]],
  ["ipa", ["ios", ".ipa"]],
  ["simulator_app_uncompressed", ["ios", ".app"]],
]);

async function directoryBytes(directory) {
  let bytes = 0;
  for (const entry of await fs.readdir(directory, { withFileTypes: true })) {
    const fullPath = path.join(directory, entry.name);
    if (entry.isDirectory()) bytes += await directoryBytes(fullPath);
    if (entry.isFile()) bytes += (await fs.stat(fullPath)).size;
  }
  return bytes;
}

async function measureArtifact(kind, file) {
  const type = artifacts.get(kind);
  if (!type || path.extname(file).toLowerCase() !== type[1])
    throw new Error("Artifact kind and extension must match");
  const stat = await fs.lstat(file);
  if (kind === "simulator_app_uncompressed" && stat.isDirectory())
    return directoryBytes(file);
  if (kind !== "simulator_app_uncompressed" && stat.isFile()) return stat.size;
  throw new Error("Unexpected artifact file type");
}

function eventId(identity) {
  const id = createHash("sha256")
    .update(JSON.stringify(identity))
    .digest()
    .subarray(0, 16);
  id[6] = (id[6] & 15) | 0x80; // UUIDv8, digest-based build identity
  id[8] = (id[8] & 63) | 0x80;
  return id
    .toString("hex")
    .replace(/^(.{8})(.{4})(.{4})(.{4})(.{12})$/, "$1-$2-$3-$4-$5");
}

async function reportArtifact(
  { file, kind, build, version, configuration, environment, host, token },
  fetcher = fetch,
) {
  if (!host && !token) return { status: "disabled" };
  const origin = new URL(host);
  if (
    origin.protocol !== "https:" ||
    origin.username ||
    origin.password ||
    origin.pathname !== "/" ||
    origin.search ||
    origin.hash ||
    !/^phc_[A-Za-z0-9_-]+$/.test(token ?? "")
  )
    throw new Error("Invalid performance destination");
  if (
    !["dev", "prod"].includes(environment) ||
    !["debug", "release"].includes(configuration) ||
    !/^[0-9]+(?:\.[0-9]+)*$/.test(build) ||
    !/^[0-9A-Za-z][0-9A-Za-z.+-]{0,63}$/.test(version ?? "")
  )
    throw new Error("Invalid build identity");
  const size = await measureArtifact(kind, file);
  if (!Number.isSafeInteger(size) || size < 1)
    throw new Error("Empty or invalid artifact size");
  const [platform] = artifacts.get(kind);
  const identity = [
    environment,
    platform,
    String(build),
    version,
    configuration,
    kind,
    size,
  ];
  const response = await fetcher(new URL("/batch/", origin), {
    method: "POST",
    redirect: "error",
    signal: AbortSignal.timeout(5000),
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({
      api_key: token,
      batch: [
        {
          event: "mobile.perf_artifact",
          uuid: eventId(identity),
          properties: {
            distinct_id: "nyxid-mobile-build",
            $process_person_profile: false,
            $geoip_disable: true,
            surface: "mobile",
            environment,
            platform,
            build_number: String(build),
            client_version: version,
            build_configuration: configuration,
            artifact_kind: kind,
            size_bytes: size,
          },
        },
      ],
    }),
  });
  if (!response.ok) throw new Error("Ingestion failed");
  return { status: "accepted", size_bytes: size };
}

// Release packaging should succeed even if optional reporting is unavailable.
async function reportReleaseArtifact(
  file,
  kind,
  build,
  version,
  environment,
  profile,
) {
  try {
    await reportArtifact({
      file,
      kind,
      build,
      version,
      environment,
      configuration: "release",
      host: profile.performanceHost,
      token: profile.performanceToken,
    });
  } catch {
    console.warn("[mobile performance] Could not report artifact size");
  }
}

module.exports = { measureArtifact, reportArtifact, reportReleaseArtifact };

if (require.main === module) {
  (async () => {
    const { values } = parseArgs({
      options: Object.fromEntries(
        ["file", "kind", "build", "version", "configuration"].map((key) => [
          key,
          { type: "string" },
        ]),
      ),
    });
    const env = loadEnv();
    if (!["dev", "prod"].includes(env.APP_ENV))
      throw new Error("Select APP_ENV");
    const prefix = env.APP_ENV.toUpperCase();
    const result = await reportArtifact({
      ...values,
      environment: env.APP_ENV,
      token: env[prefix + "_PERFORMANCE_POSTHOG_TOKEN"],
      host: env[prefix + "_PERFORMANCE_POSTHOG_HOST"],
    });
    console.log(JSON.stringify(result));
  })().catch(() => {
    console.error(
      "Artifact size report failed. Check the artifact, build arguments and destination.",
    );
    process.exitCode = 1;
  });
}
