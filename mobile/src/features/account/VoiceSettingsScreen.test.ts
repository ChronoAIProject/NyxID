import assert from "node:assert/strict";
import test from "node:test";
import { URL, fileURLToPath } from "node:url";
import { deferred, hookHarness } from "../assistant/assistantHookTestHarness";
import type {
  VoiceOption,
  VoicePreferences,
} from "../assistant/voice/voiceContracts";

type Element = { type: unknown; props: Record<string, any> };
const plain = <T>(value: T): T => JSON.parse(JSON.stringify(value));
function elements(root: Element): Element[] {
  return [
    root,
    ...[root.props.children, root.props.accessory]
      .flat(Infinity)
      .flatMap((child) =>
        child && typeof child === "object" && "props" in child
          ? elements(child)
          : [],
      ),
  ];
}

const source: VoiceOption = {
  service_id: "11111111-1111-4111-8111-111111111111",
  connection_id: "22222222-2222-4222-8222-222222222222",
  key_source: "own",
  model: "saved-live-model",
  model_label: "Saved model",
  default_model: false,
  available: true,
  unavailable_reason: null,
  billing_owner: "acting_person",
  pricing: null,
  voice: {
    protocol: "openai_live",
    models: [{ id: "saved-live-model", label: "Saved model", default: false }],
    voices: [{ id: "marin", label: "Marin" }],
    usage_source: "provider_reported",
    billing_metrics: ["voice_seconds"],
  },
};
const saved: VoicePreferences = {
  service_id: source.service_id,
  connection_id: source.connection_id,
  key_source: source.key_source,
  model: source.model,
  voice: "aube",
  input_mode: "automatic",
  language: "en",
  notify_on_completion: true,
};

function setup(preferences: VoicePreferences = saved) {
  let online = true;
  let reply: Promise<{ voice: VoicePreferences }> | undefined;
  const calls: VoicePreferences[] = [];
  const cacheWrites: Array<{ key: unknown[]; data: unknown }> = [];
  const query = {
    options: {
      data: { options: [source] },
      isSuccess: true,
      isPending: false,
      error: null as Error | null,
      refetch: async () => {},
    },
    settings: {
      data: { voice: preferences },
      isSuccess: true,
      isPending: false,
      error: null as Error | null,
      refetch: async () => {},
    },
  };
  type Mutation = {
    mutationFn(
      preferences: VoicePreferences,
    ): Promise<{ voice: VoicePreferences }>;
    onSuccess(data: { voice: VoicePreferences }): void;
  };
  let mutation: Mutation;
  const save = {
    isPending: false,
    error: null as unknown,
    mutate(preferences: VoicePreferences) {
      save.isPending = true;
      void mutation.mutationFn(preferences).then(
        (data) => {
          save.isPending = false;
          mutation.onSuccess(data);
        },
        (error: unknown) => {
          save.isPending = false;
          save.error = error;
        },
      );
    },
  };
  const environment = {
    epoch: 7,
    api: {
      savePreferences: (value: VoicePreferences) => {
        calls.push(plain(value));
        return reply ?? Promise.resolve({ voice: plain(value) });
      },
    },
  };
  const jsx = (type: unknown, props: Element["props"]) => ({ type, props });
  const harness = hookHarness<Element>({
    "react/jsx-runtime": { jsx, jsxs: jsx, Fragment: "Fragment" },
    "react-native": { Switch: "Switch" },
    "@tanstack/react-query": {
      useQuery: ({ queryKey }: { queryKey: unknown[] }) =>
        queryKey.includes("voice-options") ? query.options : query.settings,
      useQueryClient: () => ({
        setQueryData(key: unknown[], data: { voice: VoicePreferences }) {
          cacheWrites.push({ key: plain(key), data: plain(data) });
          query.settings.data = data;
        },
      }),
      useMutation: (config: Mutation) => {
        mutation = config;
        return save;
      },
    },
    "lucide-react-native": Object.fromEntries(
      ["AudioLines", "Bell", "Languages", "Mic"].map((name) => [name, name]),
    ),
    "../../components/Skeleton": { SkeletonRows: "SkeletonRows" },
    "../../components/settings/SettingsList": Object.fromEntries(
      ["SettingsGroup", "SettingsMenuRow", "SettingsPage", "SettingsRow"].map(
        (name) => [name, name],
      ),
    ),
    "../../components/workspace/WorkspaceUI": { Notice: "Notice" },
    "../../hooks/useNetworkStatus": {
      useNetworkStatus: () => ({ isConnected: online }),
    },
    "../../lib/api/assistantVoiceApi": {
      nativeVoiceEnvironment: () => environment,
    },
    "../../theme/ThemeContext": { useTheme: () => ({ colors: {} }) },
    "../assistant/voice/useDictationLanguage": {
      useDictationLanguage: () => ({
        value: "device",
        options: [],
        choose() {},
      }),
    },
  });
  const { VoiceSettingsScreen } = harness.load(
    fileURLToPath(new URL("./VoiceSettingsScreen.tsx", import.meta.url)),
  ) as {
    VoiceSettingsScreen(props: {
      route: { params: { voice: boolean } };
    }): Element;
  };
  harness.mount(() =>
    VoiceSettingsScreen({ route: { params: { voice: true } } }),
  );
  const all = () => elements(harness.value());
  const row = (label: string) => {
    const found = all().find((node) => node.props.label === label);
    assert.ok(found, `Missing settings row: ${label}`);
    return found;
  };
  const notify = () => {
    const found = all().find((node) => node.type === "Switch");
    assert.ok(found, "Missing notification switch");
    return found;
  };
  return {
    harness,
    all,
    row,
    notify,
    query,
    calls,
    cacheWrites,
    holdSave(promise: Promise<{ voice: VoicePreferences }>) {
      reply = promise;
    },
    offline() {
      online = false;
      harness.render();
    },
  };
}

test("a missing saved voice requires explicit reselection and cache confirmation before other changes", async () => {
  const f = setup();
  const result = deferred<{ voice: VoicePreferences }>();
  f.holdSave(result.promise);
  assert.equal(f.row("Voice").props.value, "aube");
  assert.match(
    f.row("Voice").props.detail,
    /aube.*no longer available.*Choose a voice/,
  );
  assert.equal(f.row("Voice").props.options.length, 1);
  assert.equal(f.row("Voice").props.disabled, false);
  assert.equal(f.row("Listening").props.disabled, true);
  assert.equal(f.notify().props.disabled, true);

  // Exercise callbacks directly too: native disabled state is not the guard.
  f.row("Listening").props.onChange("push_to_talk");
  f.notify().props.onValueChange(false);
  f.row("Voice").props.onChange("shitan");
  assert.deepEqual(f.calls, []);

  f.row("Voice").props.onChange("marin");
  const chosen = { ...saved, voice: "marin" };
  assert.deepEqual(f.calls, [chosen]);
  assert.deepEqual(f.cacheWrites, []);
  f.harness.render();
  assert.equal(f.row("Voice").props.disabled, true);
  f.row("Voice").props.onChange("marin");
  f.row("Listening").props.onChange("push_to_talk");
  f.notify().props.onValueChange(false);
  assert.equal(f.calls.length, 1);

  result.resolve({ voice: chosen });
  await f.harness.settle();
  f.harness.render();
  assert.deepEqual(f.cacheWrites, [
    { key: ["workspace", "voice-settings", 7], data: { voice: chosen } },
  ]);
  assert.equal(f.row("Voice").props.value, "marin");
  assert.equal(f.row("Voice").props.detail, undefined);
  assert.equal(f.row("Listening").props.disabled, false);
  assert.equal(f.notify().props.disabled, false);
  f.row("Listening").props.onChange("push_to_talk");
  assert.deepEqual(f.calls.at(-1), { ...chosen, input_mode: "push_to_talk" });
  await f.harness.settle();
  f.harness.unmount();
});

test("pending or failed voice queries hide cached configuration instead of saving a fallback", () => {
  for (const key of ["options", "settings"] as const) {
    for (const state of ["pending", "error"] as const) {
      const f = setup();
      f.query[key].isSuccess = false;
      f.query[key].isPending = state === "pending";
      f.query[key].error =
        state === "error" ? new Error("Could not load voice") : null;
      f.harness.render();
      assert.equal(
        f.all().some((node) => node.props.label === "Voice"),
        false,
      );
      assert.equal(
        f.all().some((node) => node.type === "Switch"),
        false,
      );
      assert.ok(
        f
          .all()
          .some((node) =>
            state === "pending"
              ? node.type === "SkeletonRows"
              : node.type === "Notice" && node.props.tone === "error",
          ),
      );
      assert.deepEqual(f.calls, []);
      assert.deepEqual(f.cacheWrites, []);
      f.harness.unmount();
    }
  }
});

test("offline cached settings disable and guard every live voice preference callback", () => {
  const f = setup({ ...saved, voice: "marin" });
  f.offline();
  assert.equal(f.row("Voice").props.disabled, true);
  assert.equal(f.row("Listening").props.disabled, true);
  assert.equal(f.notify().props.disabled, true);
  f.row("Voice").props.onChange("marin");
  f.row("Listening").props.onChange("push_to_talk");
  f.notify().props.onValueChange(false);
  assert.deepEqual(f.calls, []);
  assert.deepEqual(f.cacheWrites, []);
  assert.ok(f.all().some((node) => /offline/.test(node.props.message ?? "")));
  f.harness.unmount();
});
