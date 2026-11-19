import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { runInNewContext } from "node:vm";
import test from "node:test";
import ts from "typescript";
import { CommonActions, StackActions, StackRouter } from "@react-navigation/routers";
import { workspaceSharedEntryNavigationAction } from "./navigationHierarchy";
import { hydratedTestRouter } from "./routerTestHarness";

type Element = { type: string; props: Record<string, any> };

// Execute the real root registration without mounting native screens, then use
// React Navigation's router for the sign-in transition and request handoff.
function registrations(isAuthenticated: boolean) {
  const source = ts.createSourceFile("AppNavigator.tsx", readFileSync(
    new URL("./AppNavigator.tsx", import.meta.url), "utf8",
  ), ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
  const declaration = source.statements.find((node) =>
    ts.isFunctionDeclaration(node) && node.name?.text === "AppNavigator",
  );
  assert.ok(declaration);
  const exports: { AppNavigator?: (props: object) => Element } = {};
  const jsx = (type: string, props: Record<string, any>) => ({ type, props });
  const contexts = [
    "WorkspaceSessionContext", "WorkspaceMenuContext", "WorkspaceNavigationGuardContext",
    "WorkspaceEdgeBackContext", "WorkspaceNavigationCancelledContext",
    "WorkspacePagePaintedContext", "WorkspaceRootSwapContext",
  ];
  const screens = [
    "WorkspaceNavigator", "SettingsSheetRoute", "MachineDesktopRoute", "AuthHomeScreen",
    "SheetEntryRoute", "TermsOfServiceScreen", "PrivacyPolicyScreen",
    "DeviceLoginScreen", "AgentKeyLoginScreen",
  ];
  runInNewContext(ts.transpileModule(declaration.getText(source), {
    compilerOptions: { module: ts.ModuleKind.CommonJS, jsx: ts.JsxEmit.ReactJSX },
  }).outputText, {
    exports, require: () => ({ jsx, jsxs: jsx }),
    useAppLanguage: () => ({ t: (value: string) => value }),
    useAuthSession: () => ({ isAuthenticated, isRestoring: false }),
    useReducedMotion: () => false,
    useTheme: () => ({ colors: {} }), fonts: {},
    Stack: { Navigator: "Navigator", Group: "Group", Screen: "Screen" },
    ...Object.fromEntries(contexts.map((name) => [name, { Provider: name }])),
    ...Object.fromEntries(screens.map((name) => [name, name])),
  });
  const keys: Record<string, string> = {};
  function visit(element: Element, groupKey = "") {
    if (!element || typeof element !== "object") return;
    const key = groupKey + (element.props.navigationKey ?? "");
    if (element.type === "Screen") keys[element.props.name] = key;
    for (const child of [element.props.children].flat(Infinity)) visit(child, key);
  }
  visit(exports.AppNavigator!({}));
  return keys;
}

test("a signed-out login link survives sign-in and opens its review above the workspace", () => {
  const before = registrations(false);
  const after = registrations(true);
  const router = hydratedTestRouter(StackRouter({ initialRouteName: "Auth" }));
  const options = (keys: Record<string, string>) => ({
    routeNames: Object.keys(keys), routeParamList: {}, routeGetIdList: {},
  });
  const code = { user_code: "ABCD-EFGH" };
  let state = router.getRehydratedState({
    stale: true, index: 0, routes: [{ name: "DeviceLogin", params: code }],
  }, options(before));
  state = router.getStateForAction(state, CommonActions.navigate("Auth"), options(before))!;
  state = router.getStateForRouteNamesChange(state, {
    ...options(after),
    routeKeyChanges: Object.keys(after).filter((name) => before[name] !== after[name]),
  });
  assert.equal(state.routes[state.index]!.name, "DeviceLogin");
  assert.deepEqual(state.routes[state.index]!.params, code);
  const action = workspaceSharedEntryNavigationAction(state, "DeviceLogin", code);
  assert.ok(action);
  const next = router.getStateForAction(state, action, options(after))!;
  assert.deepEqual(next.routes.map((route) => route.name), ["Workspace", "WorkspaceSheet"]);
  assert.deepEqual(next.routes[1]!.params, { screen: "DeviceLogin", params: code, initial: true });
  const signedOut = router.getStateForRouteNamesChange(next, {
    ...options(before),
    routeKeyChanges: Object.keys(before).filter((name) => after[name] !== before[name]),
  });
  assert.deepEqual(signedOut.routes.map((route) => route.name), ["Auth"]);
});

test("abandoning a cold login link cannot restore its review after later sign-in", () => {
  const before = registrations(false);
  const after = registrations(true);
  const router = hydratedTestRouter(StackRouter({ initialRouteName: "Auth" }));
  const options = (keys: Record<string, string>) => ({ routeNames: Object.keys(keys), routeParamList: {}, routeGetIdList: {} });
  let state = router.getRehydratedState({
    stale: true, index: 0, routes: [{ name: "DeviceLogin", params: { user_code: "ABCD-EFGH" } }],
  }, options(before));
  assert.deepEqual(state.routes.map((route) => route.name), ["DeviceLogin"]);
  state = router.getStateForAction(state, StackActions.replace("Auth"), options(before))!;
  assert.deepEqual(state.routes.map((route) => route.name), ["Auth"]);
  state = router.getStateForRouteNamesChange(state, {
    ...options(after), routeKeyChanges: Object.keys(after).filter((name) => before[name] !== after[name]),
  });
  assert.deepEqual(state.routes.map((route) => route.name), ["Workspace"]);
  assert.equal(workspaceSharedEntryNavigationAction(state, "DeviceLogin", { user_code: "ABCD-EFGH" }), null);
});
