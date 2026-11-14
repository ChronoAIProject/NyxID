import assert from "node:assert/strict";
import test from "node:test";
import { runInNewContext } from "node:vm";
import {
  terminalBridgeSchema,
  terminalKeyInput,
  terminalScript,
  terminalSize,
  terminalUpgradeError,
  terminalUpgradeStatus,
  terminalUrl,
  controlInput,
  inlineTerminalNavigation,
} from "./terminalModel";
import { terminalDocument } from "./terminalDocument";
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
    { type: "input", data: "a".repeat(65537) },
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
test("offline WebView has no network permissions, navigation or external assets", () => {
  const html = terminalDocument({
    background: "#000000",
    foreground: "#ffffff",
    cursor: "#8b5cf6",
  });
  assert.match(
    html,
    /default-src 'none'; connect-src 'none'; img-src 'none'; font-src 'none'/,
  );
  assert.doesNotMatch(html, /<(?:script|img|iframe)[^>]+src=|<link[^>]+href=/i);
  assert.match(html, /window\.fetch=undefined/);
  assert.match(html, /window\.WebSocket=undefined/);
  assert.match(html, /window\.open=undefined/);
  assert.match(html, /textarea\.autocorrect='off'/);
  assert.match(html, /textarea\.spellcheck=false/);
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
  const messages = [401, 403, 404, 429, 503, 400].map(terminalUpgradeError);
  assert.equal(new Set(messages).size, 6);
  assert.match(messages[0]!, /session expired/);
  assert.match(messages[4]!, /server is unavailable/);
});
test("output batches preserve binary bytes and wait for acknowledgements; queue overflow is bounded", async () => {
  const sent: { data: string; id: number }[] = [];
  let overflow = 0;
  const output = new TerminalOutput(
    (data, id) => sent.push({ data, id }),
    () => overflow++,
  );
  output.push(Uint8Array.from([0, 27, 255, 195, 169]).buffer);
  await new Promise((resolve) => setTimeout(resolve, 25));
  assert.deepEqual(
    [...Buffer.from(sent[0]!.data, "base64")],
    [0, 27, 255, 195, 169],
  );
  output.push(new Uint8Array(17000).buffer);
  await new Promise((resolve) => setTimeout(resolve, 25));
  assert.equal(sent.length, 1);
  output.ack(999);
  await new Promise((resolve) => setTimeout(resolve, 25));
  assert.equal(sent.length, 1);
  output.ack(sent[0]!.id);
  await new Promise((resolve) => setTimeout(resolve, 25));
  assert.equal(sent.length, 2);
  assert.equal(Buffer.from(sent[1]!.data, "base64").length, 16384);
  output.push(new Uint8Array(262145).buffer);
  assert.equal(overflow, 1);
  output.clear();
});

test("native output encoding works without the browser btoa global", async () => {
  const original = globalThis.btoa;
  const sent: string[] = [];
  const output = new TerminalOutput((data) => sent.push(data), () => assert.fail("unexpected overflow"));
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

test("empty terminal frames cannot create an endless acknowledged output loop", async () => {
  const sent: { data: string; id: number }[] = [];
  const output = new TerminalOutput((data, id) => sent.push({ data, id }), () => assert.fail("unexpected overflow"));
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
  const code = [...html.matchAll(/<script>([\s\S]*?)<\/script>/g)].at(-1)![1]!;
  const posted: unknown[] = [];
  const inputs: string[] = [];
  const pastes: string[] = [];
  const writes: number[][] = [];
  let dataListener!: (input: string) => void;
  let terminal: any;
  const textarea = { setAttribute() {} };
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
      cols = 80;
      rows = 24;
      constructor() {
        terminal = this;
      }
      loadAddon() {}
      open() {}
      focus() {}
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
      write(value: Uint8Array, callback: () => void) {
        writes.push([...value]);
        callback();
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
  runInNewContext(code, context);
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
  for (const api of ["fetch", "WebSocket", "XMLHttpRequest", "open"])
    assert.equal(context.window[api], undefined);
  assert.equal((textarea as any).autocorrect, "off");
  assert.equal((textarea as any).autocapitalize, "off");
  assert.equal((textarea as any).spellcheck, false);
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
