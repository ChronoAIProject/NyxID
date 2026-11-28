const { createHash } = require("node:crypto");
const { parseArgs } = require("node:util");
const { loadEnv, performanceDestination } = require("./lib/load-env");

const { artifacts, measureArtifact } = require("./lib/artifact-size");
const { reportPackageSize } = require("./ci-package-size");

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
  const destination = performanceDestination(host, token);
  if (!destination) throw new Error("Invalid performance destination");
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
  const response = await fetcher(new URL("/batch/", destination.host), {
    method: "POST",
    redirect: "error",
    signal: AbortSignal.timeout(5000),
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({
      api_key: destination.token,
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

// The release scripts call this after producing their AAB or exported IPA.
async function reportReleaseArtifact(
  file,
  kind,
  build,
  version,
  environment,
  profile,
) {
  if (kind === "aab") await reportPackageSize("android-aab", file);
  if (kind === "ipa") await reportPackageSize("ios-ipa", file);

  // Optional telemetry must never prevent delivery of the local size report.
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
