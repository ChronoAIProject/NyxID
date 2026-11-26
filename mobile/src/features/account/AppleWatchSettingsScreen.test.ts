import assert from "node:assert/strict";
import test from "node:test";
import { setupSettingsScreen, type Element } from "./settingsTestHarness";

const ready = {
  supported: true,
  activated: true,
  paired: true,
  installed: true,
  reachable: true,
  watchCode: "482913",
};

function watchFixture(status = ready, delivery: "delivered" | "queued" = "delivered") {
  const sent: Record<string, string>[] = [];
  let listener: ((value: typeof ready) => void) | null = null;
  return {
    sent,
    emit: (value: typeof ready) => listener?.(value),
    link: {
      getStatus: async () => status,
      send: async (message: Record<string, string>) => {
        sent.push(message);
        return delivery;
      },
      clearPending: async () => {},
      addListener: (_event: string, next: (value: typeof ready) => void) => {
        listener = next;
        return { remove: () => (listener = null) };
      },
    },
  };
}

function loginCodesFixture(states: string[]) {
  const calls: string[] = [];
  return {
    calls,
    api: {
      mint: async (grant: unknown) => {
        calls.push(`mint:${JSON.stringify(grant)}`);
        return { request_id: "req-1", code: "ABCD-EFGH", expires_at: "2099-01-01T00:00:00.000Z" };
      },
      status: async (id: string) => {
        calls.push(`status:${id}`);
        const status = states.length > 1 ? states.shift()! : states[0]!;
        return {
          request_id: id,
          status,
          expires_at: "2099-01-01T00:00:00.000Z",
          redeemed_at: null,
          client_label: null,
          client_ip: null,
          client_ip_attribution: "none",
          can_revoke: false,
          auth_kind: "account_session",
        };
      },
      cancel: async (id: string) => {
        calls.push(`cancel:${id}`);
      },
    },
  };
}

function notices(all: () => Element[]) {
  return all()
    .filter((node) => node.type === "Notice")
    .map((node) => node.props.message as string);
}

async function settleUntil(
  env: ReturnType<typeof setupSettingsScreen>,
  check: () => boolean,
) {
  for (let i = 0; i < 40 && !check(); i++) {
    await new Promise((resolve) => setTimeout(resolve, 5));
    await env.harness.settle();
  }
  assert.ok(check(), "screen never reached the expected state");
}

test("connecting sends only a one-time account code and confirms the watch signed in", async () => {
  const watch = watchFixture();
  const codes = loginCodesFixture(["redeemed"]);
  const env = setupSettingsScreen("./AppleWatchSettingsScreen.tsx", "AppleWatchSettingsScreen", {
    watchLink: watch.link,
    loginCodes: codes.api,
  });
  await settleUntil(env, () => env.all().some((n) => n.props?.children === "Apple Watch paired"));
  env.row("Connect Apple Watch").props.onPress();
  await settleUntil(env, () => notices(env.all).includes("Your watch is signed in."));

  assert.equal(codes.calls[0], 'mint:{"auth_kind":"account_session"}');
  assert.deepEqual(watch.sent, [
    {
      v: "1",
      kind: "pair",
      code: "ABCD-EFGH",
      api_base_url: "https://auth.example.test/api/v1",
      match: "482913",
      frontend_url: "https://app.example.test",
      expires_at: "2099-01-01T00:00:00.000Z",
    },
  ]);
  assert.ok(env.row("Connect again"));
  env.harness.unmount();
  assert.ok(!codes.calls.includes("cancel:req-1"), "a redeemed code is never cancelled");
});

test("the iPhone shows the watch's code and can't connect until the watch shows one", async () => {
  const watch = watchFixture({ ...ready, watchCode: "" });
  const codes = loginCodesFixture(["pending"]);
  const env = setupSettingsScreen("./AppleWatchSettingsScreen.tsx", "AppleWatchSettingsScreen", {
    watchLink: watch.link,
    loginCodes: codes.api,
  });
  await settleUntil(env, () =>
    env.all().some((n) => n.props?.children === "Open NyxID on your watch to see its code."),
  );
  assert.throws(() => env.row("Code on your watch"));
  assert.equal(env.row("Connect Apple Watch").props.disabled, true);
  env.row("Connect Apple Watch").props.onPress();
  await new Promise((resolve) => setTimeout(resolve, 5));
  assert.equal(codes.calls.length, 0, "no code is minted without the watch's code");

  watch.emit(ready);
  await settleUntil(env, () => env.all().some((n) => n.props?.children === "482 913"));
  assert.ok(env.row("Code on your watch"));
  assert.notEqual(env.row("Connect Apple Watch").props.disabled, true);
  env.harness.unmount();
});

test("leaving while the watch hasn't signed in cancels the code", async () => {
  const watch = watchFixture(ready, "queued");
  const codes = loginCodesFixture(["pending"]);
  const env = setupSettingsScreen("./AppleWatchSettingsScreen.tsx", "AppleWatchSettingsScreen", {
    watchLink: watch.link,
    loginCodes: codes.api,
  });
  await settleUntil(env, () => env.all().some((n) => n.props?.children === "Apple Watch paired"));
  env.row("Connect Apple Watch").props.onPress();
  await settleUntil(env, () =>
    notices(env.all).includes("Open NyxID on your watch to finish signing in."),
  );
  env.harness.unmount();
  await new Promise((resolve) => setTimeout(resolve, 5));
  assert.ok(codes.calls.includes("cancel:req-1"));
});

test("without a paired watch there is nothing to connect", async () => {
  const watch = watchFixture({ ...ready, paired: false, installed: false });
  const env = setupSettingsScreen("./AppleWatchSettingsScreen.tsx", "AppleWatchSettingsScreen", {
    watchLink: watch.link,
    loginCodes: loginCodesFixture(["pending"]).api,
  });
  await settleUntil(env, () => env.all().some((n) => n.props?.children === "No Apple Watch paired"));
  assert.throws(() => env.row("Connect Apple Watch"));
  assert.throws(() => env.row("Sign out of Apple Watch"));
  // Pairing later in the Watch app updates the screen live.
  watch.emit(ready);
  await settleUntil(env, () => env.all().some((n) => n.props?.children === "Apple Watch paired"));
  assert.ok(env.row("Connect Apple Watch"));
  env.harness.unmount();
});

test("signing the watch out asks first and only messages the watch", async () => {
  const watch = watchFixture();
  const env = setupSettingsScreen("./AppleWatchSettingsScreen.tsx", "AppleWatchSettingsScreen", {
    watchLink: watch.link,
    loginCodes: loginCodesFixture(["pending"]).api,
  });
  await settleUntil(env, () => env.all().some((n) => n.props?.children === "Apple Watch paired"));
  env.row("Sign out of Apple Watch").props.onPress();
  const alert = env.alerts.at(-1)!;
  assert.equal(alert.title, "Sign out of your Apple Watch?");
  assert.equal(watch.sent.length, 0);
  alert.buttons!.find((button) => button.style === "destructive")!.onPress!();
  await settleUntil(env, () => notices(env.all).includes("Your watch is signed out."));
  assert.deepEqual(watch.sent, [{ v: "1", kind: "sign_out" }]);
  env.harness.unmount();
});
