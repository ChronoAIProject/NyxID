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
