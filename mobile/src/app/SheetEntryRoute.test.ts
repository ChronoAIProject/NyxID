import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { runInNewContext } from "node:vm";
import test from "node:test";
import ts from "typescript";
import { StackActions } from "@react-navigation/routers";
import { workspaceSharedEntryNavigationAction } from "./navigationHierarchy";
import { apply, fixture, selected } from "./workspaceNavigationTestHarness";

// Exercise the production redirect wrapper with the native-stack mount order:
// layout effects precede publication of the initialized navigation state.
test("P8 a cold shared-entry route waits for the root stack to initialize", () => {
  const source = readFileSync(new URL("./AppNavigator.tsx", import.meta.url), "utf8");
  const component = source.slice(
    source.indexOf("function SheetEntryRoute("),
    source.indexOf("export function AppNavigator("),
  );
  for (const name of ["DeviceLogin", "AgentKeyLogin", "TermsOfService", "PrivacyPolicy"] as const) {
    const root = apply(fixture(), StackActions.push(name));
    const entry = selected(root);
    const layout: (() => void)[] = [];
    const effects: (() => void | (() => void))[] = [];
    const actions: unknown[] = [];
    let ready = false;
    const timers = new Map<number, () => void>();
    const render = runInNewContext(
      ts.transpileModule(`${component}\nSheetEntryRoute;`, {
        compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS },
      }).outputText,
      {
        useAppLanguage() {},
        useLayoutEffect: (effect: () => void) => layout.push(effect),
        useEffect: (effect: () => void | (() => void)) => effects.push(effect),
        workspaceSharedEntryNavigationAction,
        setTimeout: (callback: () => void) => {
          timers.set(1, callback);
          return 1;
        },
        clearTimeout: (id: number) => timers.delete(id),
      },
    );
    render({ navigation: {
      getState: () => ready ? root : undefined,
      dispatch: (action: unknown) => actions.push(action),
    }, route: entry });
    for (const effect of layout) effect();
    ready = true;
    const cleanup = effects.map((effect) => effect());
    for (const callback of timers.values()) callback();
    assert.equal(actions.length, 1, `${name} must open its sheet after cold initialization`);
    assert.deepEqual(actions[0], workspaceSharedEntryNavigationAction(root, name));
    for (const cancel of cleanup) if (typeof cancel === "function") cancel();
    assert.equal(timers.size, 0, "unmount cancels any queued redirect");
  }
});
