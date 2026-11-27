import { spawnSync } from "node:child_process";
import assert from "node:assert/strict";
const result = spawnSync("docker", ["logs", "nyxid-public-test"], {
  encoding: "utf8",
});
assert.equal(result.status, 0, "Unable to inspect packaged Nginx logs");
assert.ok(
  !`${result.stdout}${result.stderr}`.includes("nyxid-ci-log-privacy-check"),
  "Visitor query leaked through request or referrer logs",
);
console.log(
  "Public log privacy gate passed: request and referrer queries omitted.",
);
