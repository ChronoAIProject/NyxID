import { describe, expect, it, vi } from "vitest";
import { validateAccountExport } from "./account-export";

const complete = '{"format":"nyxid.account-export","note":"你好"}';

describe("account export validation", () => {
  it.each([null, "identity"])("uses bounded slices when byte length matches (encoding %s)", async (encoding) => {
    const blob = new Blob([" \r\n".repeat(2000), complete, "\t".repeat(6000)]);
    const text = vi.spyOn(blob, "text").mockRejectedValue(new Error("full body read forbidden"));
    const slice = vi.spyOn(blob, "slice");
    const headers = new Headers({ "Content-Length": String(blob.size), ...(encoding ? { "Content-Encoding": encoding } : {}) });
    await validateAccountExport(blob, headers);
    expect(text).not.toHaveBeenCalled();
    expect(slice.mock.calls.length).toBeGreaterThan(2);
    for (const [start = 0, end = blob.size] of slice.mock.calls) expect(end - start).toBeLessThanOrEqual(4096);
  });

  it.each(["", " \n\t", "[]", '{"format":', '"no opening brace"}'])("rejects incomplete or non-object structure even with matching length: %s", async (body) => {
    const blob = new Blob([body]);
    const text = vi.spyOn(blob, "text");
    await expect(validateAccountExport(blob, new Headers({ "Content-Length": String(blob.size) }))).rejects.toThrow();
    expect(text).not.toHaveBeenCalled();
  });

  it.each([null, "identity"])("rejects mismatched length before reading (encoding %s)", async (encoding) => {
    const blob = new Blob([complete]);
    const text = vi.spyOn(blob, "text");
    const slice = vi.spyOn(blob, "slice");
    await expect(validateAccountExport(blob, new Headers({ "Content-Length": "1024", ...(encoding ? { "Content-Encoding": encoding } : {}) }))).rejects.toThrow("Incomplete account export");
    expect(text).not.toHaveBeenCalled();
    expect(slice).not.toHaveBeenCalled();
  });

  it.each([null, "gzip", "br", " GZip "])("parses the full JSON only without usable length (encoding %s)", async (encoding) => {
    const headers = new Headers(encoding ? { "Content-Length": "1024", "Content-Encoding": encoding } : {});
    const blob = new Blob([complete]);
    const text = vi.spyOn(blob, "text");
    await validateAccountExport(blob, headers);
    expect(text).toHaveBeenCalledOnce();
    for (const invalid of ['{"format":', '{"format":"other"}', '{}', 'null']) {
      await expect(validateAccountExport(new Blob([invalid]), headers)).rejects.toThrow();
    }
  });
});
