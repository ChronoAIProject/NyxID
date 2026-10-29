import assert from "node:assert/strict";
import { URL, fileURLToPath } from "node:url";
import test from "node:test";
import { MfaChallengeError } from "../../lib/auth/mfaChallenge";
import { ApiError } from "../../lib/api/ApiError";
import { resolveErrorMessage } from "../../lib/api/errorMessages";
import { deferred, hookHarness } from "../assistant/assistantHookTestHarness";
import type { PublicAuthConfig } from "./publicAuthConfig";
import * as emailLogin from "./emailLogin";
import * as publicAuthConfig from "./publicAuthConfig";
import * as socialCallback from "./socialCallback";

type Element = { type: unknown; props: Record<string, any> };
type LoginInput = ReturnType<typeof emailLogin.passwordLoginInput>;
type Session = {
  accessToken: string;
  refreshToken?: string;
  accessTokenExpiresAt: number;
};
const plain = <T>(value: T): T => JSON.parse(JSON.stringify(value));
const signedIn = {
  userId: "review-user",
  accessToken: "test-access-token",
  refreshToken: "test-refresh-token",
  expiresIn: 3600,
};

function elements(root: Element): Element[] {
  return [
    root,
    ...[root.props.children]
      .flat(Infinity)
      .flatMap((child) =>
        child && typeof child === "object" && "props" in child
          ? elements(child)
          : [],
      ),
  ];
}

function textContent(root: Element): string {
  return elements(root)
    .flatMap((element) => [element.props.children].flat(Infinity))
    .filter((child) => typeof child === "string")
    .join(" ");
}

function setup({
  state = "ready",
  providers = [],
  emailEnabled = true,
}: {
  state?: "ready" | "loading" | "error";
  providers?: PublicAuthConfig["socialProviders"];
  emailEnabled?: boolean;
} = {}) {
  const config: PublicAuthConfig = {
    emailAuthEnabled: emailEnabled,
    socialProviders: providers,
  };
  const calls: LoginInput[] = [];
  const sessions: Session[] = [];
  const socialUrls: string[] = [];
  const registrationUrls: string[] = [];
  const navigations: string[] = [];
  const urlListeners: Array<(event: { url: string }) => void> = [];
  let login = async (_input: LoginInput) => signedIn;
  let refetch = async () => ({ data: config, error: null as Error | null });
  let configRefetches = 0;
  const appStateListeners: Array<(state: string) => void> = [];
  const colors = new Proxy(
    { dangerTone: { bg: "#fee", border: "#f00" } },
    { get: (target, key) => Reflect.get(target, key) ?? "#111" },
  );
  const jsx = (type: unknown, props: Record<string, unknown>) => ({
    type,
    props,
  });
  const harness = hookHarness<Element>({
    "react/jsx-runtime": { jsx, jsxs: jsx, Fragment: "Fragment" },
    "expo-web-browser": {
      openBrowserAsync: async (url: string) => {
        registrationUrls.push(url);
        return { type: "dismiss" };
      },
      openAuthSessionAsync: async (url: string) => {
        socialUrls.push(url);
        return { type: "cancel" };
      },
    },
    "@react-navigation/native": { useIsFocused: () => true },
    "@tanstack/react-query": {
      useQuery: () => ({
        data: state === "loading" ? undefined : config,
        isPending: state === "loading",
        isError: state === "error",
        isFetching: state === "loading",
        refetch: () => {
          configRefetches++;
          return refetch();
        },
      }),
    },
    "react-native": {
      ...Object.fromEntries(
        [
          "ActivityIndicator",
          "Pressable",
          "ScrollView",
          "Text",
          "TextInput",
          "View",
          "KeyboardAvoidingView",
        ].map((name) => [name, name]),
      ),
      Platform: { OS: "ios" },
      StyleSheet: { create: (styles: unknown) => styles },
      AppState: {
        currentState: "active",
        addEventListener: (
          _event: string,
          listener: (state: string) => void,
        ) => {
          appStateListeners.push(listener);
          return { remove: () => {} };
        },
      },
      Linking: {
        getInitialURL: async () => null,
        addEventListener: (_event: string, listener: (event: { url: string }) => void) => {
          urlListeners.push(listener);
          return { remove: () => { urlListeners.splice(urlListeners.indexOf(listener), 1); } };
        },
      },
    },
    "react-native-svg": {
      default: "Svg",
      Path: "Path",
      Defs: "Defs",
      LinearGradient: "LinearGradient",
      Stop: "Stop",
    },
    "expo-linear-gradient": { LinearGradient: "ExpoLinearGradient" },
    "../../components/ScreenContainer": { ScreenContainer: "ScreenContainer" },
    "../../components/ToastOverlay": { ToastOverlay: "ToastOverlay" },
    "../../lib/auth/sessionStore": { authSessionGeneration: () => 0 },
    "../../lib/api/mobileApi": {
      mobileApi: {
        loginWithPassword: (input: LoginInput) => {
          calls.push(plain(input));
          return login(input);
        },
        verifyMfa: (body: {code: string; mfa_token: string}) => { calls.push({ mfaCode: body.code } as LoginInput); return Promise.resolve(signedIn); },
        getSocialAuthorizeUrl: (provider: string) =>
          `https://example.test/${provider}`,
      },
    },
    "../../lib/api/errorMessages": {
      resolveErrorMessage,
    },
    "./AuthSessionContext": {
      useAuthSession: () => ({
        signInWithSession: async (session: Session) => {
          sessions.push(plain(session));
        },
      }),
    },
    "./emailLogin": emailLogin,
    "../../lib/api/publicAuthApi": { getPublicAuthConfig: async () => config },
    "./publicAuthConfig": publicAuthConfig,
    "./socialCallback": socialCallback,
    "../../lib/telemetry": { capture: () => {} },
    "../../lib/env": { NYXID_FRONTEND_URL: "https://nyxid.example" },
    "../../theme/ThemeContext": { useTheme: () => ({ colors }) },
    "../../theme/flowStyles": { createFlowStyles: () => ({}) },
  });
  const { AuthHomeScreen } = harness.load(
    fileURLToPath(new URL("./AuthHomeScreen.tsx", import.meta.url)),
  ) as { AuthHomeScreen: (props: unknown) => Element };
  harness.mount(() => AuthHomeScreen({ navigation: { navigate: (route: string) => navigations.push(route) } }));

  function input(label: string) {
    const element = elements(harness.value()).find(
      (element) =>
        element.type === "TextInput" &&
        element.props.accessibilityLabel === label,
    );
    assert.ok(element, `Expected visible ${label} input`);
    return element;
  }
  function submit() {
    const element = elements(harness.value()).find(
      (element) =>
        element.type === "Pressable" &&
        /^(Sign In|Sign in|Verify & sign in|Signing in\.\.\.)$/.test(
          textContent(element),
        ),
    );
    assert.ok(element, "Expected email sign-in action");
    return element;
  }
  const social = () =>
    elements(harness.value()).filter(
      (element) => typeof element.props.provider === "string",
    );
  function change(label: string, value: string) {
    input(label).props.onChangeText(value);
    harness.flush();
  }
  return {
    harness,
    input,
    submit,
    social,
    change,
    calls,
    sessions,
    socialUrls,
    registrationUrls,
    navigations,
    configRefetches: () => configRefetches,
    setLogin: (handler: typeof login) => {
      login = handler;
    },
    setRefetch: (handler: typeof refetch) => {
      refetch = handler;
    },
    setEmailEnabled: (enabled: boolean) => {
      config.emailAuthEnabled = enabled;
      harness.render();
    },
    openUrl: (url: string) => {
      for (const listener of [...urlListeners]) listener({ url });
      harness.flush();
    },
    background: () => {
      for (const listener of appStateListeners) listener("background");
      harness.flush();
    },
  };
}

for (const emailEnabled of [false, true]) {
test(`email login adopts the session when registration is ${emailEnabled ? "enabled" : "disabled"}`, async (t) => {
  const screen = setup({ emailEnabled });
  t.after(screen.harness.unmount);
  assert.equal(screen.input("Password").props.secureTextEntry, true);
  assert.equal(screen.social().length, 0);
  assert.equal(screen.submit().props.disabled, true);
  screen.change("Email", " reviewer@example.test ");
  screen.change("Password", " review password ");
  assert.equal(screen.submit().props.disabled, false);
  const started = Date.now();
  screen.submit().props.onPress();
  await screen.harness.settle();
  assert.deepEqual(screen.calls, [
    {
      email: "reviewer@example.test",
      password: " review password ",
    },
  ]);
  assert.equal(screen.sessions.length, 1);
  assert.deepEqual(
    {
      accessToken: screen.sessions[0]!.accessToken,
      refreshToken: screen.sessions[0]!.refreshToken,
    },
    {
      accessToken: signedIn.accessToken,
      refreshToken: signedIn.refreshToken,
    },
  );
  assert.ok(screen.sessions[0]!.accessTokenExpiresAt >= started + 3_600_000);
  assert.ok(
    screen.sessions[0]!.accessTokenExpiresAt <= Date.now() + 3_600_000,
  );
  assert.equal(
    screen.configRefetches(),
    0,
    "Email does not start another config request",
  );
});
}

for (const emailEnabled of [false, true]) {
  for (let subset = 0; subset < 8; subset++) {
    const order = ["google", "github", "apple"] as const;
    const providers = order.filter((_, i) => (subset & (1 << i)) !== 0);
    test(`sign-in renders provider subset ${subset} with email ${emailEnabled ? "on" : "off"}`, (t) => {
      const screen = setup({ emailEnabled, providers: [...providers].reverse() });
      t.after(screen.harness.unmount);
      assert.deepEqual(screen.social().map((element) => element.props.provider), providers);
      assert.deepEqual(
        screen.social().map((element) => element.props.label),
        providers.map((provider) => `Continue with ${{ google: "Google", github: "GitHub", apple: "Apple" }[provider]}`),
      );
      const tree = elements(screen.harness.value());
      for (const label of ["Email", "Password"]) {
        assert.equal(tree.some((element) => element.props.accessibilityLabel === label), true);
      }
      assert.equal(tree.some((element) => element.type === "Pressable" && textContent(element) === "Sign In"), true);
      assert.equal(tree.some((element) => element.type === "Text" && textContent(element) === "or"), providers.length > 0);
      assert.equal(tree.some((element) => element.props.accessibilityLabel === "Create account"), emailEnabled);
      if (providers.length > 0) {
        assert.ok(tree.indexOf(screen.social().at(-1)!) < tree.indexOf(screen.input("Email")), "provider rows precede email, matching the web");
      }
    });
  }
}

for (const state of ["loading", "error"] as const) {
  test(`unavailable ${state} config keeps email login and provider retry without offering undiscovered providers`, async (t) => {
    const screen = setup({ state, providers: ["google", "github", "apple"], emailEnabled: true });
    t.after(screen.harness.unmount);
    assert.deepEqual(screen.social(), []);
    screen.input("Email");
    screen.input("Password");
    assert.equal(elements(screen.harness.value()).some((element) => element.props.accessibilityLabel === "Create account"), false);
    if (state === "error") {
      const retry = elements(screen.harness.value()).find((element) => element.type === "Pressable" && textContent(element) === "Retry sign-in options");
      assert.ok(retry);
      retry.props.onPress();
      await screen.harness.settle();
      assert.equal(screen.configRefetches(), 1);
      assert.doesNotMatch(textContent(screen.harness.value()), /still sign in with email/);
    } else {
      assert.match(textContent(screen.harness.value()), /Loading sign-in options/);
    }
  });
}

test("disabling registration keeps email credentials and social providers while hiding Create account", (t) => {
  const screen = setup({ providers: ["google", "github", "apple"] });
  t.after(screen.harness.unmount);
  screen.change("Email", "reviewer@example.test");
  screen.change("Password", "password");
  screen.setEmailEnabled(false);
  assert.equal(screen.input("Email").props.value, "reviewer@example.test");
  assert.equal(screen.input("Password").props.value, "password");
  assert.equal(screen.submit().props.disabled, false);
  assert.deepEqual(screen.social().map((element) => element.props.provider), ["google", "github", "apple"]);
  assert.equal(elements(screen.harness.value()).some((element) => element.props.accessibilityLabel === "Create account"), false);
});

test("Create account opens the configured registration page only when registration is enabled", async (t) => {
  const screen = setup({ emailEnabled: false });
  t.after(screen.harness.unmount);
  assert.deepEqual(screen.registrationUrls, []);
  screen.setEmailEnabled(true);
  const registration = elements(screen.harness.value()).find((element) => element.props.accessibilityLabel === "Create account");
  assert.ok(registration);
  assert.equal(registration.props.accessibilityRole, "link");
  assert.equal(registration.props.style.minHeight, 44);
  registration.props.onPress();
  await screen.harness.settle();
  assert.deepEqual(screen.registrationUrls, ["https://nyxid.example/register"]);
  assert.deepEqual(screen.calls, []);
  screen.setEmailEnabled(false);
  registration.props.onPress();
  await screen.harness.settle();
  assert.equal(screen.registrationUrls.length, 1, "a stale registration action cannot bypass the config gate");
});

test("registration-disabled social sign-in still verifies MFA, supports recovery and keeps legal links", async (t) => {
  const screen = setup({ emailEnabled: false, providers: ["google"] });
  t.after(screen.harness.unmount);
  screen.openUrl("nyxid://auth/social/callback#status=mfa_required&provider=google&mfa_token=synthetic-challenge");
  assert.match(textContent(screen.harness.value()), /Verify your Google sign-in/);
  assert.equal(elements(screen.harness.value()).some((element) => element.props.accessibilityLabel === "Email" || element.props.accessibilityLabel === "Password"), false);
  const recovery = elements(screen.harness.value()).find((element) => element.type === "Pressable" && textContent(element) === "Use a recovery code");
  assert.ok(recovery);
  recovery.props.onPress();
  screen.harness.flush();
  screen.change("Recovery code", "abcdef12");
  assert.equal(screen.submit().props.disabled, false);
  screen.submit().props.onPress();
  await screen.harness.settle();
  assert.deepEqual(screen.calls, [{ mfaCode: "abcdef12" }]);
  assert.equal(screen.sessions.length, 1);
  assert.equal(screen.input("Password").props.value, "");
  assert.equal(elements(screen.harness.value()).some((element) => element.props.accessibilityLabel === "Recovery code"), false);
  for (const label of ["Terms", "Privacy"]) {
    const link = elements(screen.harness.value()).find((element) => element.type === "Text" && textContent(element) === label);
    assert.ok(link);
    link.props.onPress();
  }
  assert.deepEqual(screen.navigations, ["TermsOfService", "PrivacyPolicy"]);
});

test("registration-disabled configuration preserves provider error mapping and email login", (t) => {
  const screen = setup({ emailEnabled: false, providers: ["github"] });
  t.after(screen.harness.unmount);
  screen.openUrl("nyxid://auth/social/callback?status=error&provider=github&error=social_auth_conflict");
  assert.match(textContent(screen.harness.value()), /This email is linked to another login method\./);
  assert.equal(screen.sessions.length, 0);
  screen.input("Email");
  screen.input("Password");
});

test("registration-disabled email authentication handles the actual MFA challenge before adopting a session", async (t) => {
  const screen = setup({ emailEnabled: false });
  t.after(screen.harness.unmount);
  screen.setLogin(async () => { throw new MfaChallengeError("synthetic-challenge"); });
  screen.change("Email", "reviewer@example.test");
  screen.change("Password", "password");
  screen.submit().props.onPress();
  await screen.harness.settle();
  assert.equal(screen.sessions.length, 0);
  assert.equal(screen.input("Authenticator code").props.value, "");
  assert.equal(screen.submit().props.disabled, true);
  screen.change("Authenticator code", "12");
  assert.equal(screen.submit().props.disabled, true);
  screen.change("Authenticator code", "012345");
  assert.equal(screen.submit().props.disabled, false);
  screen.submit().props.onPress();
  await screen.harness.settle();
  assert.deepEqual(screen.calls, [
    { email: "reviewer@example.test", password: "password" },
    { mfaCode: "012345" },
  ]);
  assert.equal(screen.sessions.length, 1);
});

test("failed credentials show feedback and permit retry without a false MFA prompt", async (t) => {
  const screen = setup({ emailEnabled: false });
  t.after(screen.harness.unmount);
  screen.setLogin(async () => {
    throw new ApiError({
      errorKey: "authentication_failed",
      errorCode: 2000,
      statusCode: 401,
      message: "Invalid email or password",
    });
  });
  screen.change("Email", "reviewer@example.test");
  screen.change("Password", "wrong-password");
  screen.submit().props.onPress();
  await screen.harness.settle();
  assert.match(
    textContent(screen.harness.value()),
    /Invalid email or password/,
  );
  assert.equal(screen.sessions.length, 0);
  assert.equal(
    elements(screen.harness.value()).some(
      (element) => element.props.accessibilityLabel === "Authenticator code",
    ),
    false,
  );
  screen.setLogin(async () => signedIn);
  screen.change("Password", "correct-password");
  screen.submit().props.onPress();
  await screen.harness.settle();
  assert.equal(screen.calls.length, 2);
  assert.equal(screen.sessions.length, 1);
});

test("registration-disabled email login preserves the server's inactive-account error", async (t) => {
  const screen = setup({ emailEnabled: false, providers: ["google", "github", "apple"] });
  t.after(screen.harness.unmount);
  screen.setLogin(async () => {
    throw new ApiError({ errorKey: "forbidden", errorCode: 1002, statusCode: 403, message: "Account is deactivated" });
  });
  screen.change("Email", "inactive@example.test");
  screen.change("Password", "password");
  screen.submit().props.onPress();
  await screen.harness.settle();
  assert.match(textContent(screen.harness.value()), /Account is deactivated/);
  assert.equal(screen.sessions.length, 0);
  assert.equal(screen.submit().props.disabled, false);
  assert.equal(elements(screen.harness.value()).some((element) => element.props.accessibilityLabel === "Authenticator code"), false);
  assert.deepEqual(screen.social().map((element) => element.props.provider), ["google", "github", "apple"]);
});

test("a pending email attempt blocks duplicate presses and concurrent social login", async (t) => {
  const screen = setup({ providers: ["google"] });
  t.after(screen.harness.unmount);
  const pending = deferred<typeof signedIn>();
  screen.setLogin(() => pending.promise);
  screen.change("Email", "reviewer@example.test");
  screen.change("Password", "password");
  const staleSubmit = screen.submit().props.onPress;
  const staleSocial = screen.social()[0]!.props.onPress;
  staleSubmit();
  staleSubmit();
  staleSocial();
  screen.harness.flush();
  assert.equal(screen.calls.length, 1);
  assert.equal(screen.submit().props.disabled, true);
  assert.equal(screen.input("Email").props.editable, false);
  assert.equal(screen.social()[0]!.props.disabled, true);
  assert.equal(screen.configRefetches(), 0);
  assert.deepEqual(screen.socialUrls, []);
  pending.resolve(signedIn);
  await screen.harness.settle();
  assert.equal(screen.sessions.length, 1);
});

test("social login remains configured and revalidates availability before opening OAuth", async (t) => {
  const screen = setup({ providers: ["github"] });
  t.after(screen.harness.unmount);
  assert.deepEqual(
    screen.social().map((element) => element.props.provider),
    ["github"],
  );
  screen.setRefetch(async () => ({
    data: { emailAuthEnabled: false, socialProviders: [] },
    error: null,
  }));
  screen.social()[0]!.props.onPress();
  await screen.harness.settle();
  assert.equal(screen.configRefetches(), 1);
  assert.deepEqual(screen.socialUrls, []);
  assert.match(
    textContent(screen.harness.value()),
    /GitHub sign-in is not enabled/,
  );
  assert.equal(screen.input("Password").props.editable, true);
});

test("a pending social attempt blocks concurrent email login", async (t) => {
  const screen = setup({ providers: ["apple"] });
  t.after(screen.harness.unmount);
  const pending = deferred<{ data: PublicAuthConfig; error: Error | null }>();
  screen.setRefetch(() => pending.promise);
  screen.change("Email", "reviewer@example.test");
  screen.change("Password", "password");
  const staleSubmit = screen.submit().props.onPress;
  screen.social()[0]!.props.onPress();
  staleSubmit();
  screen.harness.flush();
  assert.equal(screen.submit().props.disabled, true);
  assert.equal(screen.input("Password").props.editable, false);
  assert.deepEqual(screen.calls, []);
  pending.resolve({
    data: { emailAuthEnabled: false, socialProviders: ["apple"] },
    error: null,
  });
  await screen.harness.settle();
  assert.equal(screen.socialUrls.length, 1);
  assert.equal(screen.submit().props.disabled, false);
});

test("backgrounding clears password and authenticator data", (t) => {
  const screen = setup();
  t.after(screen.harness.unmount);
  screen.change("Email", "reviewer@example.test");
  screen.change("Password", "password");
  screen.background();
  assert.equal(screen.input("Password").props.value, "");
  assert.equal(screen.input("Email").props.value, "reviewer@example.test");
  assert.equal(screen.submit().props.disabled, true);
});
