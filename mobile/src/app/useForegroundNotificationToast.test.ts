import assert from "node:assert/strict";
import test from "node:test";
import { fileURLToPath, URL } from "node:url";
import { hookHarness } from "../features/assistant/assistantHookTestHarness";
import * as accounts from "../lib/auth/authAccount";
import type { ForegroundNotification } from "../lib/notifications/pushNotifications";

for (const challenge of [false, true]) {
  for (const next of [null, "b", "a"] as const) {
    test(`foreground ${challenge ? "Review" : "Open"} and content expire on leaving A for ${next}`, () => {
      accounts.setAuthAccountId("a");
      let receive!: (n: ForegroundNotification) => void;
      const opened: { url: string; id?: string | null }[] = [];
      const h = hookHarness<any>({
        "../lib/auth/authAccount": accounts,
        "../lib/notifications/pushNotifications": {
          setForegroundNotificationHandler: (cb: typeof receive) => {
            receive = cb;
            return () => {};
          },
        },
      });
      const { useForegroundNotificationToast } = h.load(
        fileURLToPath(
          new URL("./useForegroundNotificationToast.ts", import.meta.url),
        ),
      ) as any;
      const open = (url: string, id?: string | null) => {
        opened.push({ url, id });
      };
      h.mount(() => useForegroundNotificationToast(open, open));
      const arrival = {
        title: "Private A title",
        body: null,
        url: "nyxid://activity",
        challengeId: challenge ? "challenge-a" : null,
        type: null,
        notificationId: "notification-a",
        deeplink: "nyxid://activity",
      };
      receive(arrival);
      const before = h.flush();
      const stale = before.toast.action.onPress;
      stale();
      assert.equal(opened.length, 1);
      assert.deepEqual(opened[0], challenge ? { url: "challenge-a", id: undefined } : { url: "nyxid://activity", id: "notification-a" });
      accounts.setAuthAccountId(null);
      accounts.setAuthAccountId(next);
      const after = h.flush();
      assert.equal(after.toast, null);
      assert.notEqual(
        after.toastKey,
        before.toastKey,
        "remount empties ToastOverlay's private queue",
      );
      stale();
      assert.equal(
        opened.length,
        1,
        "retained closures never enter navigation after logout, even on re-login to A",
      );
      if (!next) {
        receive(arrival);
        assert.equal(
          h.flush().toast,
          null,
          "anonymous foreground content is discarded",
        );
      }
      h.unmount();
      accounts.setAuthAccountId(null);
    });
  }
}
