import { readFileSync } from "node:fs";
import { runInNewContext } from "node:vm";
import { afterEach, describe, expect, it, vi } from "vitest";

const source = readFileSync("../integrations/oracle/nyxid_oracle.user.js", "utf8");
type Worker = { uploadFile: (data: string, name: string) => Promise<boolean>; log: (message: string) => void;
  openConfigForm: () => void; closeConfigForm: () => void };

function harness() {
  let worker!: Worker;
  const window: Record<string, unknown> = { location: new URL("https://chatgpt.com/"), addEventListener: vi.fn() };
  window.top = window.self = window;
  // Expose the real closure only in this test. Boot timers and GM transport do
  // not run; upload, exception logging and panel rendering use shipped code.
  expect(source.trimEnd().endsWith("})();")).toBe(true);
  runInNewContext(source.replace(/\}\)\(\);\s*$/, "capture({ uploadFile, log, openConfigForm, closeConfigForm });\n})();"), {
    capture: (value: Worker) => { worker = value; },
    window, document, sessionStorage, URL, File, Event, atob,
    setTimeout: vi.fn(), console: { log: vi.fn() },
    GM_getValue: (_key: string, fallback: unknown) => fallback,
    DataTransfer: class { items = { add: vi.fn() }; files = []; },
  });
  return worker;
}

afterEach(() => { document.body.replaceChildren(); sessionStorage.clear(); });

describe("Oracle userscript log panel", () => {
  it("renders task attachment filenames and upload exceptions as literal text", async () => {
    const filename = '<img src=x onerror="globalThis.pwned=true">&task.pdf';
    const message = '</div><svg onload="globalThis.pwned=true"><script>bad()</script>&failure';
    const input = document.createElement("input");
    input.type = "file";
    Object.defineProperty(input, "files", { set() { throw new Error(message); } });
    document.body.append(input);
    const worker = harness();
    expect(await worker.uploadFile("YQ==", filename)).toBe(false);
    const panel = document.getElementById("nyxid-oracle-panel")!;
    expect(panel.textContent).toContain(`file upload: ${filename}`);
    expect(panel.textContent).toContain(`PDF file input failed: ${message}`);
    expect(panel.querySelectorAll("img, svg, script, [onerror], [onload]")).toHaveLength(0);
    worker.openConfigForm();
    worker.closeConfigForm();
    expect(panel.textContent).toContain(filename);
    expect(panel.textContent).toContain(message);
    expect(panel.querySelectorAll("img, svg, script, [onerror], [onload]")).toHaveLength(0);
  });

  it("retains ten visible log rows, their order and panel controls", () => {
    const worker = harness();
    for (let index = 0; index < 25; index++) worker.log(`entry-${index}`);
    const panel = document.getElementById("nyxid-oracle-panel")!;
    const rows = Array.from(panel.children).slice(-10);
    expect(rows.map(row => row.textContent?.split(" ").at(-1))).toEqual(
      Array.from({ length: 10 }, (_, index) => `entry-${index + 15}`),
    );
    expect(panel.querySelector("#nyxid-configure")).not.toBeNull();
    expect(panel.querySelector("#nyxid-settings")).not.toBeNull();
    expect(panel.style.background).toBe("#1d1d3a");
  });

  it("keeps the userscript metadata and reported version in sync", () => {
    const version = source.match(/^\/\/ @version\s+(\S+)$/m)![1];
    expect(source).toContain(`const SCRIPT_VERSION = "nyxid-${version}";`);
    expect(version).toBe("1.0.1");
  });
});
