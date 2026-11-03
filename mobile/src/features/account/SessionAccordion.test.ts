import assert from "node:assert/strict";
import test from "node:test";
import { URL, fileURLToPath } from "node:url";
import { hookHarness } from "../assistant/assistantHookTestHarness";
import { lightColors } from "../../theme/mobileTheme";

type E = { type: any; props: Record<string, any> };
const nodes = (n: any): E[] =>
  n?.props ? [n, ...[n.props.children].flat(Infinity).flatMap(nodes)] : [];
const text = (n: E) => [n.props.children].flat(Infinity).join("");

function render(expanded: boolean, onToggle = () => {}, onSignOut = () => {}) {
  const h = hookHarness<E>({
    "react/jsx-runtime": {
      jsx: (type: unknown, props: unknown) => ({ type, props }),
      jsxs: (type: unknown, props: unknown) => ({ type, props }),
    },
    "react-native": {
      View: "View",
      Pressable: "Pressable",
      StyleSheet: { create: (value: unknown) => value },
    },
    "lucide-react-native": {
      ChevronDown: "ChevronDown",
      ChevronUp: "ChevronUp",
    },
    "../../components/workspace/WorkspaceUI": {
      ActionButton: "ActionButton",
      Label: "Label",
    },
    "../../theme/ThemeContext": { useTheme: () => ({ colors: lightColors }) },
  });
  const { SessionAccordion } = h.load(
    fileURLToPath(new URL("./SessionAccordion.tsx", import.meta.url)),
  ) as { SessionAccordion: (props: object) => E };
  h.mount(() =>
    SessionAccordion({
      session: {
        id: "s1",
        is_current: true,
        user_agent:
          "Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X) AppleWebKit/605.1.15 Version/18.0 Mobile/15E148 Safari/604.1",
        ip_address: "203.0.113.7",
        created_at: "2026-11-03T00:00:00Z",
        expires_at: "2026-12-03T00:00:00Z",
      },
      expanded,
      onToggle,
      formatDate: (value: string) => value.slice(0, 10),
      onSignOut,
      signOutDisabled: false,
      signingOut: false,
    }),
  );
  return h;
}

test("a collapsed session shows only its device, current badge and sign-in date", () => {
  let toggled = 0;
  const h = render(false, () => toggled++);
  const all = nodes(h.value());
  const header = all.find((node) => node.type === "Pressable")!;
  assert.equal(header.props.accessibilityState.expanded, false);
  assert.match(
    header.props.accessibilityLabel,
    /^Safari on iPhone, this session/,
  );
  assert.ok(all.some((node) => text(node) === "This session"));
  assert.ok(all.some((node) => node.type === "ChevronDown"));
  assert.equal(
    all.some((node) => node.type === "ActionButton"),
    false,
  );
  assert.equal(
    all.some((node) => /203\.0\.113\.7/.test(text(node))),
    false,
  );
  header.props.onPress();
  assert.equal(toggled, 1);
  h.unmount();
});

test("an expanded session reveals network, expiry and its sign-out action", () => {
  let signedOut = 0;
  const h = render(
    true,
    () => {},
    () => signedOut++,
  );
  const all = nodes(h.value());
  assert.ok(all.some((node) => node.type === "ChevronUp"));
  assert.ok(all.some((node) => text(node) === "Network: 203.0.113.7"));
  assert.ok(all.some((node) => text(node) === "Expires 2026-12-03"));
  const signOut = all.find((node) => node.type === "ActionButton")!;
  assert.equal(signOut.props.label, "Sign out on this device");
  signOut.props.onPress();
  assert.equal(signedOut, 1);
  h.unmount();
});
