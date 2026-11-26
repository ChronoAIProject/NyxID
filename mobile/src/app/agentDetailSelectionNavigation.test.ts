import assert from "node:assert/strict";
import test from "node:test";
import { readFileSync } from "node:fs";
import { runInNewContext } from "node:vm";
import ts from "typescript";

test("AgentDetail Keep editing resyncs only agentId so the same incoming link changes params again", () => {
  const source = readFileSync(
    new URL("./AppNavigator.tsx", import.meta.url),
    "utf8",
  );
  const wrapper = source.slice(
    source.indexOf("function AgentDetailRoute("),
    source.indexOf("function AgentCreateRoute("),
  );
  const exports: {
    AgentDetailRoute?: (props: unknown) => {
      props: { onSelectionKept: (id: string) => void };
    };
  } = {};
  runInNewContext(
    ts.transpileModule(
      `${wrapper}\nexports.AgentDetailRoute = AgentDetailRoute;`,
      {
        compilerOptions: {
          jsx: ts.JsxEmit.ReactJSX,
          module: ts.ModuleKind.CommonJS,
        },
      },
    ).outputText,
    {
      exports,
      AgentsScreen: "AgentsScreen",
      // Route wrappers subscribe to the app language for re-rendering.
      useAppLanguage: () => ({ t: (phrase: string) => phrase }),
      useOpenWorkspaceChat: () => () => {},
      require: () => ({
        jsx: (type: unknown, props: unknown) => ({ type, props }),
      }),
    },
  );
  const params = { agentId: "B" };
  const updates: string[] = [];
  const rendered = exports.AgentDetailRoute!({
    route: { params },
    navigation: {
      setParams: (next: typeof params) => {
        Object.assign(params, next);
        updates.push(params.agentId);
      },
    },
  });
  rendered.props.onSelectionKept("A");
  assert.deepEqual(params, { agentId: "A" });
  assert.deepEqual(updates, ["A"]);
  assert.notEqual(
    params.agentId,
    "B",
    "a later B link changes the route again",
  );
});
