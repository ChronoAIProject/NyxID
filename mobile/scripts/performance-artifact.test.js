const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs/promises");
const os = require("node:os");
const path = require("node:path");
const { measureArtifact, reportArtifact } = require("./performance-artifact");

async function temp(t) {
  const root = await fs.mkdtemp(path.join(os.tmpdir(), "artifact-size-"));
  t.after(() => fs.rm(root, { recursive: true, force: true }));
  return root;
}

test("measures archives in bytes and omits symlinks from simulator app trees", async (t) => {
  const root = await temp(t);
  for (const kind of ["aab", "apk", "ipa"]) {
    const file = path.join(root, "demo." + kind);
    await fs.writeFile(file, Buffer.alloc(3073));
    assert.equal(await measureArtifact(kind, file), 3073);
  }
  const app = path.join(root, "Demo.app");
  await fs.mkdir(path.join(app, "bundle"), { recursive: true });
  await fs.writeFile(path.join(app, "binary"), Buffer.alloc(5));
  await fs.writeFile(path.join(app, "bundle", "asset"), Buffer.alloc(4));
  await fs.symlink(root, path.join(app, "external"));
  assert.equal(await measureArtifact("simulator_app_uncompressed", app), 9);
  await assert.rejects(measureArtifact("ipa", app));
});

test("reports only build metadata and repeats the same event id on retry", async (t) => {
  const file = path.join(await temp(t), "demo.apk");
  await fs.writeFile(file, Buffer.alloc(35));
  const input = {
    file,
    kind: "apk",
    build: "24",
    version: "1.0.2",
    environment: "dev",
    configuration: "debug",
    host: "https://example.org",
    token: "phc_test",
  };
  const ids = [];
  const send = async (url, options) => {
    assert.equal(url.href, "https://example.org/batch/");
    assert.equal(options.redirect, "error");
    const {
      batch: [event],
    } = JSON.parse(options.body);
    assert.equal(event.event, "mobile.perf_artifact");
    ids.push(event.uuid);
    assert.deepEqual(event.properties, {
      distinct_id: "nyxid-mobile-build",
      $process_person_profile: false,
      $geoip_disable: true,
      surface: "mobile",
      environment: "dev",
      platform: "android",
      build_number: "24",
      client_version: "1.0.2",
      build_configuration: "debug",
      artifact_kind: "apk",
      size_bytes: 35,
    });
    return { ok: true };
  };
  for (let i = 0; i < 2; i++)
    assert.deepEqual(await reportArtifact(input, send), {
      status: "accepted",
      size_bytes: 35,
    });
  assert.equal(ids[0], ids[1]);
  assert.deepEqual(await reportArtifact({}), { status: "disabled" });
  await assert.rejects(
    reportArtifact({ ...input, host: "http://example.org" }, send),
    /destination/,
  );
  await assert.rejects(
    reportArtifact(input, async () => ({ ok: false })),
    /Ingestion/,
  );
});

test("CI reports exact file bytes and only warns above the budget", async (t) => {
  const { spawnSync } = require("node:child_process");
  const root = await temp(t);
  const file = path.join(root, "build.ipa");
  const summary = path.join(root, "summary.md");
  const run = (profile, artifact = file) =>
    spawnSync(
      process.execPath,
      [path.join(__dirname, "ci-package-size.js"), profile, artifact],
      {
        encoding: "utf8",
        env: {
          ...process.env,
          GITHUB_ACTIONS: "true",
          GITHUB_STEP_SUMMARY: summary,
        },
      },
    );

  await fs.writeFile(file, "");
  assert.notEqual(run("ios-reference-ipa").status, 0);
  assert.notEqual(run("ios-ipa", path.join(root, "missing.ipa")).status, 0);
  assert.notEqual(run("android-aab").status, 0);
  assert.notEqual(run("unknown-profile").status, 0);
  await assert.rejects(fs.stat(summary), { code: "ENOENT" });

  const budget = 30 * 1024 * 1024;
  await fs.truncate(file, budget);
  const atLimit = run("ios-reference-ipa");
  assert.equal(atLimit.status, 0, atLimit.stderr);
  assert.match(atLimit.stdout, /Within budget/);
  assert.equal(atLimit.stderr, "");

  await fs.truncate(file, budget + 1);
  const aboveLimit = run("ios-reference-ipa");
  assert.equal(aboveLimit.status, 0, aboveLimit.stderr);
  assert.match(
    aboveLimit.stderr,
    /^::warning title=Mobile package size::.*Over budget/m,
  );
  const markdown = await fs.readFile(summary, "utf8");
  assert.match(
    markdown,
    /iOS unsigned device IPA \(CI reference\) \| 30\.00 MiB \| 31457280 \| 30 MiB \| Within budget/,
  );
  assert.match(markdown, /31457281 \| 30 MiB \| Over budget/);
});

test("standard release scripts can report AAB and signed IPA size with telemetry disabled", async (t) => {
  const { reportReleaseArtifact } = require("./performance-artifact");
  const root = await temp(t);
  const summary = path.join(root, "release.md");
  const previous = process.env.GITHUB_STEP_SUMMARY;
  process.env.GITHUB_STEP_SUMMARY = summary;
  t.after(() => {
    if (previous === undefined) delete process.env.GITHUB_STEP_SUMMARY;
    else process.env.GITHUB_STEP_SUMMARY = previous;
  });

  for (const kind of ["aab", "ipa"]) {
    const file = path.join(root, "release." + kind);
    await fs.writeFile(file, Buffer.alloc(2048));
    await reportReleaseArtifact(file, kind, "12", "1.0.4", "prod", {});
  }
  const markdown = await fs.readFile(summary, "utf8");
  assert.match(markdown, /Android release AAB \| 0\.00 MiB \| 2048 \| 110 MiB/);
  assert.match(markdown, /iOS signed IPA \| 0\.00 MiB \| 2048 \| 25 MiB/);
});
