// @vitest-environment node
import { expect, it } from "vitest";
import { readFile } from "node:fs/promises";
// @ts-expect-error Build-only module is intentionally JavaScript.
import { applicationPaths } from "../scripts/public-routing.mjs";
it("derives exact application routes without treating arbitrary namespaces as valid", async () => {
  const paths = applicationPaths(
    await readFile(new URL("../src/router.tsx", import.meta.url), "utf8"),
  );
  expect(paths).toContain("/keys/$keyId");
  expect(paths).toContain("/orgs/$orgId/service-accounts/$saId");
  expect(paths).toContain("/admin/users/$userId");
  expect(paths).not.toContain("/admin/$");
  expect(paths).not.toContain("/settings/$");
  expect(paths).not.toContain("/oauthConsentPreview");
});
