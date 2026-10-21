import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { expect, it } from "vitest";

type Case = { project: string; file: string; title: string };
type Suite = { specs?: { file: string; title: string; tests: { projectName: string }[] }[]; suites?: Suite[] };

it("pins every case selected by the complete production Playwright configuration", () => {
  const required = JSON.parse(readFileSync("e2e-production/required-cases.json", "utf8")) as Case[];
  const report = JSON.parse(execFileSync(process.execPath, [
    "node_modules/@playwright/test/cli.js", "test", "--config=playwright.production.config.ts", "--list", "--reporter=json",
  ], { cwd: resolve("."), env: { ...process.env, CI: "true", NYXID_PRODUCTION_TEST_URL: "http://127.0.0.1:8080" }, encoding: "utf8" })) as { suites: Suite[]; errors: unknown[] };
  const actual: Case[] = [];
  const walk = (suite: Suite) => {
    for (const spec of suite.specs ?? []) {
      for (const test of spec.tests) actual.push({ project: test.projectName, file: spec.file, title: spec.title });
    }
    for (const child of suite.suites ?? []) walk(child);
  };
  report.suites.forEach(walk);
  const identities = (cases: Case[]) => cases.map(({ project, file, title }) => JSON.stringify([project, file, title])).sort();
  expect(report.errors).toEqual([]);
  expect(identities(actual)).toEqual(identities(required));
  expect(new Set(identities(required)).size).toBe(required.length);
});
