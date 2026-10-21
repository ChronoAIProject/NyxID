import assert from "node:assert/strict";
import { createHash } from "node:crypto";
const hash = (value) => createHash("sha256").update(value).digest("hex");
export function verifyReleaseConfiguration(release) {
  assert.equal(
    release.configRevision,
    hash(JSON.stringify(release.configuration)),
    "Effective configuration revision mismatch",
  );
  assert.equal(
    release.contentRevision,
    hash(
      JSON.stringify({
        pages: release.pages,
        visibility: release.visibility,
        configRevision: release.configRevision,
      }),
    ),
    "Release content/configuration revision mismatch",
  );
}
export function retainedDisabledFeatureCode(text, visibility) {
  return [
    ["newsletter", visibility.newsletter],
    ["contact-chat", visibility.contactChat],
    ["footer-status", visibility.footerStatus],
  ]
    .filter(
      ([name, enabled]) =>
        !enabled &&
        new RegExp(`["']data-website-feature["']\\s*:\\s*["']${name}["']`).test(
          text,
        ),
    )
    .map(([name]) => name);
}
