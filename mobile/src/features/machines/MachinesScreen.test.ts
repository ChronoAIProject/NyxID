import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { URL, fileURLToPath } from "node:url";
import { runInNewContext } from "node:vm";
import {
  CommonActions,
  StackActions,
  StackRouter,
} from "@react-navigation/routers";
import ts from "typescript";

type Effect = () => void;

function linkedMachine() {
  const options = {
    routeNames: ["Assistant", "Machines", "MachineDetail"],
    routeParamList: {},
    routeGetIdList: {},
  };
  const router = StackRouter({ initialRouteName: "Assistant" });
  const initial = router.getInitialState(options);
  const linked = router.getRehydratedState(
    {
      stale: true,
      index: 1,
      routes: [
        { key: initial.routes[0]!.key, name: initial.routes[0]!.name },
        {
          key: "qa-machines",
          name: "Machines",
          params: { machine: "qa-mac", tab: "logins" },
        },
      ],
    },
    options,
  );
  let state = initial;
  const unhandled: string[] = [];
  const layouts: Effect[] = [];
  const passive: Effect[] = [];
  const dispatch = (action: Parameters<typeof router.getStateForAction>[1]) => {
    const next = router.getStateForAction(state, action, options);
    if (next) state = router.getRehydratedState(next, options);
    else unhandled.push(action.type);
  };
  const jsx = (type: unknown, props: unknown) => ({ type, props });
  const mocks: Record<string, unknown> = {
    react: {
      useLayoutEffect: (effect: Effect) => layouts.push(effect),
      useEffect: (effect: Effect) => passive.push(effect),
      useMemo: (factory: () => unknown) => factory(),
      useState: () => [undefined, () => {}],
    },
    "react/jsx-runtime": { jsx, jsxs: jsx, Fragment: "Fragment" },
    "@tanstack/react-query": { useQuery: () => ({ isPending: true }) },
    "../../app/WorkspaceHeader": { useWorkspaceBack: () => {} },
    "../../components/SegmentControl": { SegmentControl: "Segments" },
    "../../components/Skeleton": { SkeletonRows: "Skeleton" },
    "../../components/settings/SettingsList": {
      SettingsGroup: "Group",
      SettingsRow: "Row",
    },
    "../../components/workspace/WorkspaceUI": {
      Notice: "Notice",
      WorkspacePage: "Page",
    },
    "../../hooks/usePullRefresh": { usePullRefresh: () => ({}) },
    "./SavedLogins": { SavedLogins: "SavedLogins" },
    "./MachineDesktopEntries": { MachineDesktopEntries: "Desktop" },
    "./MachinePresentation": { machineTabs: [], machinePageContent: {} },
    "../../lib/api/machinesApi": { machinesApi: {} },
    "./machineContracts": {},
    "./MachineUI": {
      machineKeys: () => ["machines"],
      useMachineRuntime: () => ({ active: false, epoch: 1 }),
    },
  };
  const filename = fileURLToPath(
    new URL("./MachinesScreen.tsx", import.meta.url),
  );
  const source = ts.transpileModule(readFileSync(filename, "utf8"), {
    compilerOptions: {
      module: ts.ModuleKind.CommonJS,
      target: ts.ScriptTarget.ES2022,
      jsx: ts.JsxEmit.ReactJSX,
    },
  }).outputText;
  const exports: Record<string, any> = {};
  runInNewContext(
    source,
    {
      exports,
      require: (name: string) => {
        assert.ok(name in mocks, `Unexpected dependency: ${name}`);
        return mocks[name];
      },
    },
    { filename },
  );
  function render(route = linked.routes[1]!) {
    layouts.length = 0;
    passive.length = 0;
    exports.MachinesScreen({
      route,
      navigation: {
        getState: () => state,
        setParams: (params: object) =>
          dispatch({ ...CommonActions.setParams(params), source: route.key }),
        push: (name: string, params: object) =>
          dispatch({ ...StackActions.push(name, params), source: route.key }),
        goBack: () => dispatch(CommonActions.goBack()),
      },
    });
  }
  return {
    render,
    layouts,
    passive,
    unhandled,
    state: () => state,
    commit: (next = linked) => {
      state = next;
    },
    initial,
    linked,
    back: () => dispatch(CommonActions.goBack()),
  };
}

test("machine deep links wait for the parent commit and retain a consumed list on Back", () => {
  const x = linkedMachine();
  x.render();
  // A newly rendered child runs layout effects before its navigator commits
  // the linked routes and latest getState callback. Exercise that ordering.
  x.layouts.forEach((run) => run());
  assert.deepEqual(x.unhandled, []);
  assert.equal(x.state(), x.initial);
  x.commit();
  x.passive.forEach((run) => run());
  assert.deepEqual(x.unhandled, []);
  assert.deepEqual(
    x.state().routes.map((route) => route.name),
    ["Assistant", "Machines", "MachineDetail"],
  );
  assert.equal(
    (x.state().routes[2]!.params as { machineId: string }).machineId,
    "qa-mac",
  );
  assert.equal(x.state().routes[0]!.key, x.initial.routes[0]!.key);
  assert.equal(x.state().routes[1]!.key, x.linked.routes[1]!.key);
  x.back();
  const params = x.state().routes[1]!.params as {
    machine?: string;
    tab?: string;
  };
  assert.equal(params.machine, undefined);
  assert.equal(params.tab, "logins");
  x.render(x.state().routes[1]!);
  x.layouts.forEach((run) => run());
  x.passive.forEach((run) => run());
  assert.equal(x.state().routes.length, 2);
});

test("a replayed machine-link effect opens the detail only once", () => {
  const x = linkedMachine();
  x.render();
  x.layouts.forEach((run) => run());
  x.commit();
  x.passive.forEach((run) => {
    run();
    run();
  });
  assert.deepEqual(x.unhandled, []);
  assert.equal(x.state().routes.length, 3);
});

test("an old link effect cannot consume a newer machine parameter or a removed route", () => {
  for (const removed of [false, true]) {
    const x = linkedMachine();
    x.render();
    x.layouts.forEach((run) => run());
    x.commit(
      removed
        ? x.initial
        : {
            ...x.linked,
            routes: x.linked.routes.map((route, index) =>
              index === 1
                ? { ...route, params: { machine: "newer-mac" } }
                : route,
            ),
          },
    );
    const committed = x.state();
    x.passive.forEach((run) => run());
    assert.deepEqual(x.unhandled, []);
    assert.equal(x.state(), committed);
  }
});
