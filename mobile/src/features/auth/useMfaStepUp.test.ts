import assert from "node:assert/strict";
import test from "node:test";
import { URL, fileURLToPath } from "node:url";
import { ApiError } from "../../lib/api/ApiError";
import { deferred, hookHarness } from "../assistant/assistantHookTestHarness";

function setup() {
  let onState!: (state: string) => void;
  let generation = 1;
  let retries = 0;
  const verification = deferred<void>();
  const requests: any[] = [];
  const harness = hookHarness<any>({
    "react/jsx-runtime": {
      jsx: (type: unknown, props: unknown) => ({ type, props }),
    },
    "react-native": {
      AppState: {
        addEventListener: (_event: string, callback: typeof onState) => {
          onState = callback;
          return { remove() {} };
        },
      },
    },
    "../../lib/api/http": {
      requestJson: async (...args: any[]) => {
        requests.push(args);
        await verification.promise;
      },
    },
    "../../lib/api/mobileApi": {
      mobileApi: {
        getAccountProfile: async () => ({
          capabilities: { enabled_features: ["auth:mobile-mfa"] },
        }),
      },
    },
    "../../lib/auth/sessionStore": { authSessionGeneration: () => generation },
    "../../components/PrimaryButton": { PrimaryButton: "PrimaryButton" },
    "../../theme/ThemeContext": { useTheme: () => ({ colors: {} }) },
    "./deviceLoginStyles": { createDeviceLoginStyles: () => ({}) },
  });
  const module = harness.load(
    fileURLToPath(new URL("./useMfaStepUp.tsx", import.meta.url)),
  ) as { useMfaStepUp: (scope: string) => any };
  harness.mount(() => module.useMfaStepUp("login"));
  return {
    harness,
    verification,
    requests,
    state: (value: string) => {
      onState(value);
      harness.flush();
    },
    changeAccount: () => {
      generation++;
      harness.render();
    },
    retries: () => retries,
    prompt: async () => {
      await harness
        .value()
        .handle(
          new ApiError({
            statusCode: 403,
            errorCode: 2003,
            errorKey: "mfa_step_up_required",
            message: "Verify this session",
          }),
          async () => {
            retries++;
          },
        );
      harness.flush();
      assert.ok(harness.value().prompt);
      return harness.value().prompt;
    },
  };
}

test("step-up stays guarded through verification and retry and a bad code cannot invalidate the session", async () => {
  const env = setup();
  const prompt = await env.prompt();
  const pending = prompt.props.verify("123456");
  env.harness.flush();
  assert.equal(env.harness.value().busy, true);
  assert.equal(env.requests[0][1].invalidateSessionOnAuthFailure, false);
  env.verification.resolve();
  await pending;
  env.harness.flush();
  assert.equal(env.harness.value().busy, false);
  assert.equal(env.harness.value().prompt, null);
  assert.equal(env.retries(), 1);
  env.harness.unmount();
});
test("backgrounding clears the prompt and fences a late verification even after foregrounding", async () => {
  const env = setup();
  const prompt = await env.prompt();
  const pending = prompt.props.verify("123456");
  env.state("background");
  assert.equal(env.harness.value().prompt, null);
  env.state("active");
  env.verification.resolve();
  await pending;
  env.harness.flush();
  assert.equal(env.retries(), 0);
  assert.equal(env.harness.value().busy, false);
  env.harness.unmount();
});
test("account replacement discards the pending step-up without retrying its write", async () => {
  const env = setup();
  const prompt = await env.prompt();
  const pending = prompt.props.verify("123456");
  env.changeAccount();
  assert.equal(env.harness.value().prompt, null);
  env.verification.resolve();
  await pending;
  assert.equal(env.retries(), 0);
  env.harness.unmount();
});
