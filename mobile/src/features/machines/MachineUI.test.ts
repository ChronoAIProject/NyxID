import assert from "node:assert/strict";
import test from "node:test";
import { URL, fileURLToPath } from "node:url";
import { deferred, hookHarness } from "../assistant/assistantHookTestHarness";
import * as contracts from "./machineContracts";
import { ApiError } from "../../lib/api/ApiError";

function setup() {
  let epoch = 1;
  let focused = true;
  const app = {
    currentState: "active",
    addEventListener: () => ({ remove() {} }),
  };
  const harness = hookHarness<any>({
    "./machineContracts": contracts,
    "react-native": { AppState: app, Alert: { alert() {} } },
    "@react-navigation/native": { useIsFocused: () => focused },
    "expo-web-browser": {},
    zod: { z: { ZodError: class {} } },
    "../../hooks/useNetworkStatus": {
      useNetworkStatus: () => ({ isConnected: true }),
    },
    "../../lib/auth/authEpoch": { getAuthEpoch: () => epoch },
    "../../lib/api/ApiError": { ApiError },
    "../../lib/env": { NYXID_FRONTEND_URL: "https://example.test" },
  });
  const module = harness.load(
    fileURLToPath(new URL("./MachineUI.tsx", import.meta.url)),
  ) as { useMachineWrite: () => any };
  harness.mount(() => module.useMachineWrite());
  return {
    harness,
    app,
    setEpoch: (value: number) => {
      epoch = value;
    },
    blur: () => {
      focused = false;
      harness.render();
    },
  };
}
test("machine writes synchronously lock double submission and fence account/background changes", async () => {
  const x = setup();
  const gate = deferred<void>();
  let calls = 0;
  let current!: () => boolean;
  const write = x.harness.value();
  const first = write.run(async (guard: () => boolean) => {
    calls++;
    current = guard;
    await gate.promise;
  });
  await write.run(async () => {
    calls++;
  });
  assert.equal(calls, 1);
  x.app.currentState = "background";
  assert.equal(current(), false);
  x.app.currentState = "active";
  x.setEpoch(2);
  assert.equal(current(), false);
  gate.resolve();
  await first;
  x.harness.unmount();
});
test("uncertain writes pause until explicit review and never retry automatically", async () => {
  const x = setup();
  let calls = 0;
  const oldCallback = x.harness.value().run;
  await oldCallback(async () => {
    calls++;
    throw new Error("Connection lost");
  });
  await oldCallback(async () => {
    calls++;
  });
  assert.equal(calls, 1);
  x.harness.flush();
  assert.equal(x.harness.value().uncertain, true);
  await x.harness.value().run(async () => {
    calls++;
  });
  assert.equal(calls, 1);
  x.harness.value().reviewed();
  x.harness.flush();
  assert.equal(x.harness.value().uncertain, false);
  x.harness.unmount();
});
test("structured forbidden machine errors preserve code without becoming uncertain or signing out", async () => {
  const x = setup();
  await x.harness.value().run(async () => {
    throw new ApiError({
      errorKey: "machine_permission_revoked",
      errorCode: 12420,
      statusCode: 403,
      message: "Permission revoked",
    });
  });
  x.harness.flush();
  assert.match(x.harness.value().error, /12420/);
  assert.equal(x.harness.value().uncertain, false);
  x.harness.unmount();
});
