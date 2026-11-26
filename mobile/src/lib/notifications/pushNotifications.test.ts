import assert from "node:assert/strict";
import test from "node:test";
import { fileURLToPath, URL } from "node:url";
import { hookHarness, deferred } from "../../features/assistant/assistantHookTestHarness";
import * as destination from "./notificationDestination";

test("actual foreground receive listener survives malformed-relative URLs and supplies type/Activity Open fallbacks", async () => {
  let receive!: (notification: any) => void;
  const h = hookHarness(
    {
      "./notificationDestination": destination,
      "../env": {
        NYXID_APP_SCHEME: "nyxid",
        NYXID_TRUSTED_FRONTEND_URLS: ["https://nyx.example"],
        NYXID_TRUSTED_API_BASE_URLS: [],
      },
      "expo-notifications": {
        setNotificationHandler: () => {},
        registerTaskAsync: async () => {},
        addNotificationReceivedListener: (cb: typeof receive) => {
          receive = cb;
          return { remove: () => {} };
        },
      },
      "expo-task-manager": {
        isTaskDefined: () => true,
        isTaskRegisteredAsync: async () => true,
      },
      "expo-secure-store": {},
      "react-native": {
        AppState: { currentState: "active" },
        Platform: { OS: "ios" },
      },
      "../api/mobileApi": {},
      "./approvalRefreshSignal": {
        signalApprovalStateMayHaveChanged: () => {},
      },
    },
    { __DEV__: false },
  );
  const module = h.load(
    fileURLToPath(new URL("./pushNotifications.ts", import.meta.url)),
  ) as typeof import("./pushNotifications");
  const displayed: any[] = [];
  const unsubscribe = module.setForegroundNotificationHandler((n) =>
    displayed.push(n),
  );
  const cleanup = await module.initializeNotificationRuntime();
  for (const type of ["channel_bot_failed", "channel_bot_restored", "unknown"]) {
    receive({
      request: {
        content: { title: "Review update", data: { type, url: "//[" } },
      },
    });
  }
  assert.deepEqual(
    displayed.map((n) => n.url),
    [
      "nyxid://plugins?tab=connectors",
      "nyxid://plugins?tab=connectors",
      "nyxid://activity",
    ],
  );
  unsubscribe();
  cleanup();
});

test('R1-A old logout timeout stays with its owner while the replacement account registers', async () => {
  let actor: string | null = null, expire!: () => void, suppressed = false;
  const entered = deferred<void>(), deleteReply = deferred<void>(), nativeUnregisters: Array<string | null> = [], registered: Array<string | null> = [];
  const store = new Map([['nyxid.push.device_token','synthetic-device']]);
  const notifications = {
    unregisterForNotificationsAsync: async () => {nativeUnregisters.push(actor)},
    dismissAllNotificationsAsync:async()=>{},setBadgeCountAsync:async()=>{},
    clearLastNotificationResponseAsync:async()=>{},
    getPermissionsAsync:async()=>({status:'granted',granted:true}),
    getDevicePushTokenAsync:async()=>({type:'ios',data:'synthetic-device'})
  };
  const h=hookHarness({
    '../auth/authAccount':{getAuthAccountId:()=>actor}, '../env':{}, './notificationDestination':{},
    'expo-notifications':notifications, 'expo-task-manager':{isTaskDefined:()=>true},
    'react-native':{Platform:{OS:'ios'},AppState:{currentState:'active'}},
    'expo-secure-store':{getItemAsync:async (k: string)=>store.get(k)??null,setItemAsync:async(k: string,v: string)=>store.set(k,v),deleteItemAsync:async (k: string)=>store.delete(k)},
    '../api/mobileApi':{mobileApi:{unregisterPushToken:async(_body: unknown,bearer: string)=>{assert.equal(bearer,'synthetic-a');entered.resolve();await deleteReply.promise},
      registerPushToken:async()=>{registered.push(actor);return {registered:true}}}},
    './pushDeliveryState':{beginPushDeliveryStateHydration:()=>{},pushDeliveryStateHydrated:async()=>{},
      setPushDeliverySuppressed:(v: boolean)=>{suppressed=v},isPushDeliverySuppressed:()=>suppressed}
  },{__DEV__:false, setTimeout:(fn: () => void,ms: number)=>{if(ms===10000){expire=fn;return {}}return setTimeout(fn,ms)},clearTimeout});
  const push=h.load(fileURLToPath(new URL('./pushNotifications.ts', import.meta.url))) as typeof import('./pushNotifications');
  const old = push.deactivatePushOnLogout(await push.capturePushLogout('synthetic-a'));
  await entered.promise;
  actor = 'b';
  const replacement = push.activatePushAfterLogin({forceRegister:true});
  expire(); // old DELETE response is still outstanding after the 10 s queue budget
  await old;
  assert.deepEqual(nativeUnregisters, [], 'old timeout must not unregister B while its activation is queued');
  deleteReply.resolve();
  const result=await replacement;
  console.log('replacement push activation', {nativeUnregisters, registered, result:result.registered, suppressed});
  assert.ok(result.registered, 'replacement login must complete its own push registration');
  assert.deepEqual(registered, ['b']);
  assert.equal(suppressed, false);
});
