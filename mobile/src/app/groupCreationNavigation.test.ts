import assert from "node:assert/strict";
import test from "node:test";
import { readFileSync } from "node:fs";
import { runInNewContext } from "node:vm";
import ts from "typescript";
import { StackActions } from "@react-navigation/routers";
import {
  apply,
  backContent,
  content,
  fixture,
  pushContent,
} from "./workspaceNavigationTestHarness";

test("creating a crew replaces its editor from Crews, sidebar and cold-start origins; edit-save goes back", () => {
  const source = readFileSync(
    new URL("./AppNavigator.tsx", import.meta.url),
    "utf8",
  );
  const wrapper = source.slice(
    source.indexOf("function GroupEditorRoute("),
    source.indexOf("function AgentDetailRoute("),
  );
  const exports: {
    GroupEditorRoute?: (props: unknown) => {
      props: { onSaved: (id: string) => void };
    };
  } = {};
  runInNewContext(
    ts.transpileModule(
      `${wrapper}\nexports.GroupEditorRoute = GroupEditorRoute;`,
      {
        compilerOptions: {
          jsx: ts.JsxEmit.ReactJSX,
          module: ts.ModuleKind.CommonJS,
        },
      },
    ).outputText,
    {
      exports,
      // Route wrappers subscribe to the app language for re-rendering.
      useAppLanguage: () => ({ t: (phrase: string) => phrase }),
      GroupEditorScreen: "GroupEditorScreen",
      require: () => ({
        jsx: (type: unknown, props: unknown) => ({ type, props }),
      }),
    },
  );
  for (const origin of ["crews", "sidebar", "cold-start", "editing"]) {
    const initial =
      origin === "crews"
        ? pushContent(fixture(), "Agents", { tab: "groups" })
        : fixture();
    let state = pushContent(
      initial,
      "GroupEditor",
      origin === "editing" ? { groupId: "crew" } : undefined,
    );
    const before = content(initial).routes.map((route) => route.name);
    const rendered = exports.GroupEditorRoute!({
      route: { params: origin === "editing" ? { groupId: "crew" } : undefined },
      navigation: {
        goBack: () => {
          state = backContent(state);
        },
        replace: (name: string, params: object) => {
          state = apply(state, {
            ...StackActions.replace(name, params),
            target: content(state).key,
          });
        },
      },
    });
    rendered.props.onSaved("crew");
    assert.deepEqual(
      content(state).routes.map((route) => route.name),
      origin === "editing" ? before : [...before, "GroupChat"],
    );
    if (origin !== "editing") {
      state = backContent(state);
      assert.deepEqual(
        content(state).routes.map((route) => route.name),
        before,
      );
    }
  }
});
