// CI wiring fixture only: never a production enablement or task acceptance.
import assert from "node:assert/strict";
import fs from "node:fs/promises";
const release = JSON.parse(
  await fs.readFile("dist/public-release.json", "utf8"),
);
assert.equal(release.sourceCommit, process.env.SOURCE_COMMIT);
assert.deepEqual(release.configuration, {
  blogMenu: false,
  newsletter: null,
  contactChat: null,
  footerStatus: null,
  solutions: { "help-desk": "/assistant?setup=help-desk&draft=hello+world" },
});
assert.deepEqual(release.visibility, {
  blogMenu: false,
  newsletter: false,
  contactChat: false,
  footerStatus: false,
  solutions: ["help-desk"],
});
let browserStats;
if (process.argv.includes("--browser")) {
  const results = JSON.parse(
    await fs.readFile("dist/public-browser-results.json", "utf8"),
  );
  browserStats = results.stats;
  assert.equal(browserStats.expected, 2);
  assert.equal(browserStats.skipped, 0);
  assert.equal(browserStats.unexpected, 0);
  assert.equal(browserStats.flaky, 0);
}
await fs.writeFile(
  "dist/solution-fixture.json",
  JSON.stringify(
    {
      kind: "single-solution-wiring-fixture",
      productionDelivery: false,
      realTaskAccepted: false,
      sourceCommit: release.sourceCommit,
      contentRevision: release.contentRevision,
      configRevision: release.configRevision,
      configuration: release.configuration,
      browserStats,
    },
    null,
    2,
  ),
);
console.log(
  "Single help-desk fixture configuration verified; not production delivery or real task acceptance.",
);
