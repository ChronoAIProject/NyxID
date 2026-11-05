import { expect, it } from "vitest";
import { parseCrmImport } from "./import";
it("roundtrips a bounded exported page and preserves sparse JSON omissions", () => {
  const records = [{ name: "A" }, { name: "B", domain: null }];
  const rows = parseCrmImport(
    JSON.stringify({
      object: { key: "company" },
      records,
      total_count: 2,
      next_cursor: null,
    }),
  );
  expect(rows).toEqual(records);
  expect(Object.hasOwn(rows[0]!, "domain")).toBe(false);
  expect([...new Set(rows.flatMap(Object.keys))]).toEqual(["name", "domain"]);
});
it("parses quoted CSV and rejects inconsistent/beyond-bound input", () => {
  expect(
    parseCrmImport('name,domain\r\n"A, Inc",a.test\r\n"B\nCompany",b.test'),
  ).toEqual([
    { name: "A, Inc", domain: "a.test" },
    { name: "B\nCompany", domain: "b.test" },
  ]);
  expect(() => parseCrmImport("name,domain\nA")).toThrow(/wrong number/);
  expect(() =>
    parseCrmImport(
      JSON.stringify(Array.from({ length: 101 }, () => ({ name: "A" }))),
    ),
  ).toThrow(/100/);
  expect(() =>
    parseCrmImport(
      JSON.stringify([
        { name: "A" },
        Object.fromEntries(
          Array.from({ length: 101 }, (_, i) => [`column${i}`, 1]),
        ),
      ]),
    ),
  ).toThrow(/100 columns/);
});
