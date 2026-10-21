// @vitest-environment node
import { expect, it } from "vitest";
import { createHash } from "node:crypto";
// @ts-expect-error Build-only module.
import {
  retainedDisabledFeatureCode,
  verifyReleaseConfiguration,
} from "../scripts/public-visibility-contract.mjs";
const hash = (value: string) =>
  createHash("sha256").update(value).digest("hex");
it("still rejects retained newsletter/chat code when Status alone is enabled", () => {
  expect(
    retainedDisabledFeatureCode(
      `{"data-website-feature":"newsletter"};{"data-website-feature":"contact-chat"};{"data-website-feature":"footer-status"}`,
      { newsletter: false, contactChat: false, footerStatus: true },
    ),
  ).toEqual(["newsletter", "contact-chat"]);
});
it("rejects configuration or content revision drift", () => {
  const release = {
    configuration: { newsletter: { endpoint: "/api/first" } },
    visibility: { newsletter: true },
    pages: [],
    configRevision: "",
    contentRevision: "",
  };
  release.configRevision = hash(JSON.stringify(release.configuration));
  release.contentRevision = hash(
    JSON.stringify({
      pages: release.pages,
      visibility: release.visibility,
      configRevision: release.configRevision,
    }),
  );
  expect(() => verifyReleaseConfiguration(release)).not.toThrow();
  expect(() =>
    verifyReleaseConfiguration({
      ...release,
      configuration: { newsletter: { endpoint: "/api/second" } },
    }),
  ).toThrow(/revision/);
  expect(() => verifyReleaseConfiguration({ ...release, pages: [{}] })).toThrow(
    /revision/,
  );
});
