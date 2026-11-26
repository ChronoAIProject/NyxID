import assert from "node:assert/strict";
import test from "node:test";
import { runInNewContext } from "node:vm";
import {
  terminalBridgeSchema,
  terminalCommandSchema,
  terminalKeyInput,
  terminalScript,
  terminalSize,
  terminalUpgradeError,
  terminalUpgradeStatus,
  terminalUrl,
  controlInput,
  inlineTerminalNavigation,
  truncateTerminalPaste,
} from "./terminalModel";
import {
  escapeInlineScript,
  terminalBridgeScript,
  terminalDocument,
} from "./terminalDocument";
import { TerminalOutput } from "./terminalOutput";

test("terminal URL uses API scheme, preserves exact target and encodes principal with bounded dimensions", () => {
  const url = new URL(
    terminalUrl(
      "https://api.example/api/v1/",
      "service/id",
      "ops+user@example",
      900,
      1,
    ),
  );
  assert.equal(url.protocol, "wss:");
  assert.equal(url.pathname, "/api/v1/ssh/service%2Fid/terminal");
  assert.equal(url.searchParams.get("principal"), "ops+user@example");
  assert.equal(url.searchParams.get("cols"), "500");
  assert.equal(url.searchParams.get("rows"), "2");
  assert.equal(
    new URL(terminalUrl("http://localhost:3001/api/v1", "id", "root")).protocol,
    "ws:",
  );
  assert.deepEqual(terminalSize(NaN, Infinity), { cols: 80, rows: 24 });
  assert.equal(
    [...url.searchParams.keys()].some((key) =>
      /token|authorization/i.test(key),
    ),
    false,
  );
});
test("terminal key bar respects application cursor mode and sticky Ctrl mappings", () => {
  for (const [key, suffix] of [
    ["ArrowUp", "A"],
    ["ArrowDown", "B"],
    ["ArrowRight", "C"],
    ["ArrowLeft", "D"],
  ] as const) {
    assert.equal(terminalKeyInput(key, false), `\x1b[${suffix}`);
    assert.equal(terminalKeyInput(key, true), `\x1bO${suffix}`);
  }
  assert.equal(terminalKeyInput("Esc", false), "\x1b");
  assert.equal(terminalKeyInput("Tab", false), "\t");
  assert.equal(terminalKeyInput("|", false), "|");
  assert.equal(controlInput("c"), "\x03");
  assert.equal(controlInput("["), "\x1b");
  assert.equal(controlInput("hello"), "hello");
});
test("bridge messages are strict and bounded; command serialization cannot inject JavaScript", () => {
  for (const value of [
    { type: "input", data: "a".repeat(65549) },
    { type: "resize", cols: 1, rows: 2 },
    { type: "resize", cols: 80, rows: NaN },
    { type: "navigate", url: "https://evil.example" },
    { type: "ready", token: "secret" },
  ])
    assert.equal(terminalBridgeSchema.safeParse(value).success, false);
  assert.equal(
    terminalBridgeSchema.safeParse({ type: "input", data: "ls\r" }).success,
    true,
  );
  assert.equal(
    terminalBridgeSchema.safeParse({ type: "input", data: "x".repeat(65548) })
      .success,
    true,
  );
  assert.equal(
    terminalBridgeSchema.safeParse({ type: "input", data: "x".repeat(65549) })
      .success,
    false,
  );
  assert.equal(
    terminalCommandSchema.safeParse({
      type: "paste",
      text: "x".repeat(65536),
    }).success,
    true,
  );
  assert.equal(
    terminalCommandSchema.safeParse({
      type: "session-ended",
      message: "x".repeat(8192),
    }).success,
    true,
  );
  for (const command of [
    { type: "session-ended", message: "x".repeat(8193) },
    { type: "session-ended", message: "Closed", token: "secret" },
    { type: "session-ended" },
  ]) assert.equal(terminalCommandSchema.safeParse(command).success, false);
  const text = "');window.fetch('https://evil.example');</script>";
  let received: unknown;
  runInNewContext(terminalScript({ type: "paste", text }), {
    window: {
      nyxTerminal: (value: unknown) => {
        received = value;
      },
    },
  });
  assert.equal((received as { text: string }).text, text);
  assert.throws(() =>
    terminalScript({ type: "output", data: "';alert(1)", id: 1 }),
  );
});
test("inline script escaping neutralizes closing tags without changing other text", () => {
  for (const [value, expected] of [
    ["</script>", "<\\/script>"],
    ["</SCRIPT>", "<\\/SCRIPT>"],
    ["</ScRiPt", "<\\/ScRiPt"],
    ["İ </ScRiPt> </div> </SCRIPT>", "İ <\\/ScRiPt> </div> <\\/SCRIPT>"],
    [
      "plain text <script> </style> < /script>",
      "plain text <script> </style> < /script>",
    ],
    ["", ""],
  ] as const)
    assert.equal(escapeInlineScript(value), expected);
});
test("offline WebView has no network permissions, navigation or external assets", () => {
  const html = terminalDocument({
    background: "#000000",
    foreground: "#ffffff",
    cursor: "#8b5cf6",
  });
  assert.match(
    html,
    /default-src 'none'; connect-src 'none'; img-src 'none'; font-src data:/,
  );
  const lowercaseHtml = html.toLowerCase();
  for (const [tag, attribute] of [
    ["script", "src="],
    ["img", "src="],
    ["iframe", "src="],
    ["link", "href="],
  ] as const) {
    let from = 0;
    for (
      let start = lowercaseHtml.indexOf(`<${tag}`, from);
      start !== -1;
      start = lowercaseHtml.indexOf(`<${tag}`, from)
    ) {
      const end = lowercaseHtml.indexOf(">", start);
      assert.notEqual(end, -1);
      assert.equal(lowercaseHtml.slice(start, end).includes(attribute), false);
      from = end + 1;
    }
  }
  assert.match(html, /window\.fetch=undefined/);
  assert.match(html, /window\.WebSocket=undefined/);
  assert.match(html, /window\.open=undefined/);
  assert.match(html, /JetBrains Mono/);
  assert.match(html, /fontFamily:"'JetBrains Mono', monospace"/);
  assert.match(html, /setAttribute\('autocorrect','off'\)/);
  assert.match(html, /setAttribute\('spellcheck','false'\)/);
  assert.match(html, /term\.paste\(message.text\)/);
  assert.match(html, /applicationCursorKeysMode/);
  assert.doesNotMatch(html, /Bearer |accessToken|Authorization:/);
  const navigation = inlineTerminalNavigation();
  assert.equal(
    navigation({
      url: "about:blank",
      navigationType: "other",
      isTopFrame: true,
    }),
    true,
  );
  for (const url of [
    "about:blank",
    "https://api.example",
    "file:///tmp/x",
    "nyxid://services",
  ])
    assert.equal(navigation({ url }), false);
  assert.equal(
    inlineTerminalNavigation()({ url: "about:blank", navigationType: "click" }),
    false,
  );
});
test("upgrade status copy distinguishes auth, permissions, missing service, capacity and server errors", () => {
  const messages = [401, 403, 404, 429, 503, 400].map((status) =>
    terminalUpgradeError(status),
  );
  assert.equal(new Set(messages).size, 6);
  assert.match(messages[0]!, /session expired/);
  assert.match(messages[4]!, /server is unavailable/);
  assert.equal(messages[1], "You don't have permission to open this terminal.");
  assert.equal(
    terminalUpgradeError(403, true),
    "The server refused this session. An approval may have been denied or expired, or you may no longer have access. Reconnect to try again.",
  );
  for (const status of [401, 404, 429, 503, 400])
    assert.equal(terminalUpgradeError(status, true), terminalUpgradeError(status));
});
test("output batches preserve binary bytes and wait for acknowledgements; queue overflow is bounded", async () => {
  const sent: { data: string; id: number }[] = [];
  const output = new TerminalOutput(
    (data, id) => sent.push({ data, id }),
    () => Uint8Array.from([24, 27, 91, 48, 109, 13, 10]),
  );
  output.push(Uint8Array.from([0, 27, 255, 195, 169]).buffer);
  await new Promise((resolve) => setTimeout(resolve, 25));
  assert.deepEqual(
    [...Buffer.from(sent[0]!.data, "base64")],
    [0, 27, 255, 195, 169],
  );
  output.push(new Uint8Array(70000).buffer);
  await new Promise((resolve) => setTimeout(resolve, 25));
  assert.equal(sent.length, 1);
  output.ack(999);
  await new Promise((resolve) => setTimeout(resolve, 25));
  assert.equal(sent.length, 1);
  output.ack(sent[0]!.id);
  await new Promise((resolve) => setTimeout(resolve, 25));
  assert.equal(sent.length, 2);
  assert.equal(Buffer.from(sent[1]!.data, "base64").length, 65536);
  output.push(new Uint8Array(4 * 1024 * 1024 + 1).buffer);
  assert.ok(output.queuedBytes <= 4 * 1024 * 1024);
  output.clear();
});

test("output overflow drops queued oldest bytes and emits one gap marker before newest data", async () => {
  const marker = Uint8Array.from([24, 27, 91, 48, 109, 13, 10]);
  const sent: { data: string; id: number }[] = [];
  const output = new TerminalOutput(
    (data, id) => sent.push({ data, id }),
    () => marker,
  );
  const first = new Uint8Array(65_536).fill(1);
  output.push(first.buffer);
  await new Promise((resolve) => setTimeout(resolve, 25));
  const newest = new Uint8Array(65_536).fill(2);
  for (let index = 0; index < 65; index += 1) output.push(newest.buffer.slice(0));
  output.push(Uint8Array.of(9).buffer);
  assert.ok(output.queuedBytes <= 4 * 1024 * 1024);

  const delivered: number[] = [];
  let acknowledged = 0;
  while (acknowledged < sent.length || output.queuedBytes > 0) {
    const batch = sent[acknowledged];
    if (batch) {
      delivered.push(...Buffer.from(batch.data, "base64"));
      output.ack(batch.id);
      acknowledged += 1;
    }
    await new Promise((resolve) => setTimeout(resolve, 25));
  }
  const markerAt = delivered.findIndex((value, index) =>
    marker.every((byte, offset) => delivered[index + offset] === byte),
  );
  assert.ok(markerAt >= 0);
  assert.equal(
    delivered.filter((_, index) =>
      marker.every((byte, offset) => delivered[index + offset] === byte),
    ).length,
    1,
  );
  assert.equal(delivered.at(-1), 9);
  output.clear();
});

test("native output encoding works without the browser btoa global", async () => {
  const original = globalThis.btoa;
  const sent: string[] = [];
  const output = new TerminalOutput((data) => sent.push(data), () => Uint8Array.of(24));
  try {
    Object.defineProperty(globalThis, "btoa", { configurable: true, writable: true, value: undefined });
    const bytes = Uint8Array.from([0, 27, 255, 195, 169]);
    output.push(bytes.buffer);
    await new Promise((resolve) => setTimeout(resolve, 25));
    assert.deepEqual([...Buffer.from(sent[0]!, "base64")], [...bytes]);
  } finally {
    output.clear();
    Object.defineProperty(globalThis, "btoa", { configurable: true, writable: true, value: original });
  }
});

test("clipboard truncation preserves code points and reports a boundary", () => {
  const exact = "a".repeat(65_536);
  assert.deepEqual(truncateTerminalPaste(exact), { text: exact, truncated: false });
  const split = "a".repeat(65_535) + "😀" + "tail";
  const result = truncateTerminalPaste(split);
  assert.equal(result.truncated, true);
  assert.equal(result.text, "a".repeat(65_535));
  assert.doesNotMatch(result.text, /\uD800|\uDC00/);
});

test("empty terminal frames cannot create an endless acknowledged output loop", async () => {
  const sent: { data: string; id: number }[] = [];
  const output = new TerminalOutput(
    (data, id) => sent.push({ data, id }),
    () => Uint8Array.of(24),
  );
  try {
    output.push(new ArrayBuffer(0));
    await new Promise((resolve) => setTimeout(resolve, 25));
    assert.equal(sent.length, 0);
    output.push(Uint8Array.of(65).buffer);
    await new Promise((resolve) => setTimeout(resolve, 25));
    assert.equal(sent.length, 1);
    assert.equal(Buffer.from(sent[0]!.data, "base64").toString(), "A");
    output.ack(sent[0]!.id);
    await new Promise((resolve) => setTimeout(resolve, 25));
    assert.equal(sent.length, 1);
  } finally {
    output.clear();
  }
});

test("the executed offline bridge emits real escape keys, uses xterm paste, and exposes no network APIs", () => {
  const html = terminalDocument({
    background: "#000000",
    foreground: "#ffffff",
    cursor: "#9775fa",
  });
  assert.equal(html.includes("<script>" + terminalBridgeScript + "</script>"), true);
  const posted: unknown[] = [];
  const inputs: string[] = [];
  const pastes: string[] = [];
  const writes: (number[] | string)[] = [];
  let dataListener!: (input: string) => void;
  let terminal: any;
  const attributes: Record<string, string> = {};
  const textarea = {
    disabled: false,
    setAttribute(name: string, value: string) {
      attributes[name] = value;
    },
  };
  const context = {
    window: {
      ReactNativeWebView: {
        postMessage: (raw: string) => {
          posted.push(terminalBridgeSchema.parse(JSON.parse(raw)));
        },
      },
      addEventListener() {},
    } as any,
    document: {
      addEventListener() {},
      getElementById() {
        return {};
      },
      querySelector() {
        return textarea;
      },
    },
    Terminal: class {
      modes = { applicationCursorKeysMode: false };
      options = { disableStdin: false, cursorBlink: true };
      focused = false;
      resets = 0;
      cols = 80;
      rows = 24;
      constructor() {
        terminal = this;
      }
      loadAddon() {}
      open() {}
      focus() { this.focused = true; }
      blur() { this.focused = false; }
      reset() { this.resets++; }
      onData(listener: typeof dataListener) {
        dataListener = listener;
      }
      input(value: string) {
        inputs.push(value);
        dataListener(value);
      }
      paste(value: string) {
        pastes.push(value);
        dataListener(`\x1b[200~${value}\x1b[201~`);
      }
      write(value: Uint8Array | string, callback?: () => void) {
        writes.push(typeof value === "string" ? value : [...value]);
        callback?.();
      }
    },
    FitAddon: {
      FitAddon: class {
        fit() {}
      },
    },
    ResizeObserver: class {
      constructor(_listener: unknown) {}
      observe() {}
    },
    clearTimeout() {},
    setTimeout() {},
    atob,
  };
  runInNewContext(terminalBridgeScript, context);
  for (const [key, expected] of [
    ["Esc", "\x1b"],
    ["Tab", "\t"],
    ["ArrowUp", "\x1b[A"],
  ] as const) {
    runInNewContext(terminalScript({ type: "key", key }), context);
    assert.equal(inputs.at(-1), expected);
  }
  terminal.modes.applicationCursorKeysMode = true;
  runInNewContext(terminalScript({ type: "key", key: "ArrowUp" }), context);
  assert.equal(inputs.at(-1), "\x1bOA");
  runInNewContext(
    terminalScript({ type: "paste", text: "echo hello\n" }),
    context,
  );
  assert.deepEqual(pastes, ["echo hello\n"]);
  assert.deepEqual(posted.at(-1), {
    type: "input",
    data: "\x1b[200~echo hello\n\x1b[201~",
  });
  runInNewContext(
    terminalScript({ type: "output", data: "ABv/", id: 1 }),
    context,
  );
  assert.deepEqual(writes, [[0, 27, 255]]);
  assert.deepEqual(posted.at(-1), { type: "ack", id: 1 });
  const serverError = "Connection refused.\nCheck the target.\x1b[31m\x07";
  runInNewContext(
    terminalScript({ type: "session-ended", message: serverError }),
    context,
  );
  assert.equal(
    writes.at(-1),
    "\r\n\x1b[0m\x1b[2mConnection refused.\r\nCheck the target.[31m\x1b[0m\r\n\x1b[?25l",
  );
  assert.equal(terminal.options.disableStdin, true);
  assert.equal(terminal.options.cursorBlink, false);
  assert.equal(terminal.focused, false);
  assert.equal(textarea.disabled, true);
  const postedBefore = posted.length;
  const inputsBefore = inputs.length;
  runInNewContext(terminalScript({ type: "key", key: "Tab" }), context);
  runInNewContext(terminalScript({ type: "paste", text: "ls\r" }), context);
  dataListener("typed after closure");
  assert.equal(posted.length, postedBefore);
  assert.equal(inputs.length, inputsBefore);
  assert.equal(terminal.focused, false);
  runInNewContext(terminalScript({ type: "reset" }), context);
  assert.equal(terminal.options.disableStdin, false);
  assert.equal(terminal.resets, 1);
  assert.equal(textarea.disabled, false);
  runInNewContext(terminalScript({ type: "key", key: "Tab" }), context);
  assert.deepEqual(posted.at(-1), { type: "input", data: "\t" });
  for (const api of ["fetch", "WebSocket", "XMLHttpRequest", "open"])
    assert.equal(context.window[api], undefined);
  assert.deepEqual(attributes, {
    autocorrect: "off",
    autocapitalize: "off",
    autocomplete: "off",
    spellcheck: "false",
  });
});

test("paste stays within the validated bridge boundary after bracketed markers", () => {
  const text = "x".repeat(65536);
  const parsed = terminalBridgeSchema.parse({
    type: "input",
    data: `\x1b[200~${text}\x1b[201~`,
  });
  if (parsed.type !== "input") throw new Error("expected input bridge message");
  assert.equal(parsed.data.length, 65548);
  assert.throws(() =>
    terminalBridgeSchema.parse({
      type: "input",
      data: `\x1b[200~${text}x\x1b[201~`,
    }),
  );
});

test("native upgrade diagnostics survive RN's empty error event and close-reason delivery", () => {
  for (const message of [
    "Expected HTTP 101 response but was '401 Unauthorized'",
    "received bad response code: 401",
    "Unexpected server response: 401",
  ]) {
    assert.equal(terminalUpgradeStatus({ message }), 401);
  }
  assert.equal(terminalUpgradeStatus({ message: "Socket closed" }), undefined);
  assert.equal(terminalUpgradeStatus({ status: 403 }), 403);
});
