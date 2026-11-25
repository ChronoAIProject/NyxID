import { englishTranslation, richTranslationMock } from "../../i18n/translationTestHarness";
// Native modules are replaced at the boundary; the actual hook source and its
// local state/session helpers execute unchanged. Effects commit after render.
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import { runInNewContext } from "node:vm";
import ts from "typescript";
import { memo } from "react";

type Effect = {
  dependencies?: unknown[];
  cleanup?: () => void;
  run: () => void | (() => void);
};
const same = (a?: unknown[], b?: unknown[]) =>
  !!a && !!b && a.length === b.length && a.every((v, i) => Object.is(v, b[i]));

export function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((yes, no) => {
    resolve = yes;
    reject = no;
  });
  return { promise, resolve, reject };
}

export function hookHarness<T>(
  mocks: Record<string, unknown>,
  globals: Record<string, unknown> = {},
) {
  // Rich translation slots contain real element objects (links/emphasis). Execute
  // that pure mock at the JSX boundary so traversals see the same native leaves.
  const jsxRuntime = mocks["react/jsx-runtime"] as Record<string, unknown> | undefined;
  if (jsxRuntime) {
    const runtime = { ...jsxRuntime };
    for (const name of ["jsx", "jsxs"]) {
      const factory = runtime[name] as ((...args: any[]) => unknown) | undefined;
      if (factory) runtime[name] = (type: unknown, props: any, ...args: unknown[]) =>
        type === richTranslationMock.Translation
          ? richTranslationMock.Translation(props)
          : factory(type, props, ...args);
    }
    mocks["react/jsx-runtime"] = runtime;
  }
  // Share the VM Error constructor across loaded production modules. Errors
  // created by host-side fixtures retain the pre-existing unknown-error path.
  const realmError = runInNewContext("Error") as ErrorConstructor;
  const slots: unknown[] = [];
  const effects = new Map<number, Effect>();
  let pendingEffects: number[] = [];
  let cursor = 0;
  let dirty = false;
  let renderHook: () => T;
  let value: T;
  const react = {
    memo,
    createContext: <V>(value: V) => ({ value, Provider: "ContextProvider" }),
    useContext: <V>(context: { value: V }) => context.value,
    useRef: <V>(initial: V) => {
      const i = cursor++;
      slots[i] ??= { current: initial };
      return slots[i] as { current: V };
    },
    useState: <V>(initial: V | (() => V)) => {
      const i = cursor++;
      if (!(i in slots))
        slots[i] =
          typeof initial === "function" ? (initial as () => V)() : initial;
      return [
        slots[i],
        (next: V | ((previous: V) => V)) => {
          const updated =
            typeof next === "function"
              ? (next as (previous: V) => V)(slots[i] as V)
              : next;
          if (!Object.is(updated, slots[i])) {
            slots[i] = updated;
            dirty = true;
          }
        },
      ] as const;
    },
    useEffect: (run: Effect["run"], dependencies?: unknown[]) => {
      const i = cursor++;
      const previous = effects.get(i);
      if (!previous || !same(previous.dependencies, dependencies)) {
        effects.set(i, { dependencies, run, cleanup: previous?.cleanup });
        pendingEffects.push(i);
      }
    },
    useLayoutEffect: (run: Effect["run"], dependencies?: unknown[]) => {
      react.useEffect(run, dependencies);
    },
    useCallback: <V>(callback: V, dependencies: unknown[]) => {
      const i = cursor++;
      const previous = slots[i] as
        | { callback: V; dependencies: unknown[] }
        | undefined;
      if (!previous || !same(previous.dependencies, dependencies))
        slots[i] = { callback, dependencies };
      return (slots[i] as { callback: V }).callback;
    },
    useMemo: <V>(factory: () => V, dependencies: unknown[]) => {
      const i = cursor++;
      const previous = slots[i] as
        | { value: V; dependencies: unknown[] }
        | undefined;
      if (!previous || !same(previous.dependencies, dependencies))
        slots[i] = { value: factory(), dependencies };
      return (slots[i] as { value: V }).value;
    },
  };
  function flush() {
    let count = 0;
    while (dirty || pendingEffects.length) {
      if (++count > 50) throw new Error("Hook did not settle");
      if (dirty) {
        dirty = false;
        cursor = 0;
        value = renderHook();
      }
      const committing = pendingEffects;
      pendingEffects = [];
      for (const i of committing) effects.get(i)?.cleanup?.();
      for (const i of committing) {
        const effect = effects.get(i)!;
        effect.cleanup = effect.run() || undefined;
      }
    }
    return value;
  }
  let slabMotionMock: Record<string, unknown> | undefined;
  let floatingInsetMock: Record<string, unknown> | undefined;
  const harness = {
    mock: (name: string, value: unknown) => {
      mocks[name] = value;
    },
    load: (filename: string): Record<string, unknown> => {
      const require = createRequire(filename);
      const output = ts.transpileModule(readFileSync(filename, "utf8"), {
        compilerOptions: {
          module: ts.ModuleKind.CommonJS,
          target: ts.ScriptTarget.ES2022,
          jsx: ts.JsxEmit.ReactJSX,
          esModuleInterop: true,
        },
      }).outputText;
      const exports: Record<string, unknown> = {};
      runInNewContext(
        output,
        {
          exports,
          require: (name: string) =>
            name === "react"
              ? react
              : name in mocks
                ? mocks[name]
                : (name.endsWith("/i18n/useLocalizedState") || name.endsWith("/i18n/localizedError"))
                  ? harness.load(require.resolve(name))
                : name.endsWith("/i18n/appTranslation") && Object.entries(mocks).some(([key]) => key.endsWith("/i18n/LanguageProvider"))
                  ? { ...require(name), translateApp: (phrase: string, values?: Record<string, string | number>) =>
                      englishTranslation((Object.entries(mocks).find(([key]) => key.endsWith("/i18n/LanguageProvider"))![1] as { useAppLanguage(): { t: typeof englishTranslation } }).useAppLanguage().t(phrase, values), values) }
                : name.endsWith("/performance/useNativePerformance")
                  ? {
                      usePerformanceLifecycle: () => {},
                      useChatNativeFrames: () => ({ beginDrag() {}, endDrag() {}, momentumBegin() {}, momentumEnd() {} }),
                    }
                : name.endsWith("/deviceLanguages")
                  ? harness.load(require.resolve(name))
                : name.endsWith("/DynamicTypeText")
                  ? // The font-change leaf itself is covered by DynamicTypeText.test.
                    {
                      DynamicTypeText: (
                        mocks["react-native"] as { Text: unknown }
                      ).Text,
                    }
                  : name.endsWith("/WorkspaceSlabMotion")
                    ? // The UI-thread animation leaf: a plain view here,
                      // covered by the shell test.
                      (slabMotionMock ??= {
                        HEADER_ACTION_ENTERING: undefined,
                        HEADER_ACTION_EXITING: undefined,
                        MotionView: (mocks["react-native"] as { View: unknown })
                          .View,
                        RailInsetView: "RailInsetView",
                        RailShortcutsView: "RailShortcutsView",
                        RailShortcutIcon: "RailShortcutIcon",
                      })
                  : name.endsWith("/workspaceFloatingInset")
                    ? (floatingInsetMock ??= {
                        WorkspaceFloatingInsetContext: react.createContext(0),
                        WorkspaceRailMotionContext: react.createContext(null),
                        useFloatingTrailingInset: () => 0,
                        useRailScrollIndicator: () => ({}),
                      })
                  : name.endsWith("/FloatingControl")
                    ? // The one glass primitive (native glass); tested in
                      // FloatingControl.test.
                      {
                        FloatingCapsule: "FloatingCapsule",
                        FloatingGroup: "FloatingGroup",
                        FloatingButton: "FloatingButton",
                        FloatingItem: "FloatingItem",
                        floatingGlassAvailable: () => true,
                        useFloatingSurfaceColor: () => "floating-surface",
                      }
                  : name.endsWith("/SettingsHeaderButton")
                    ? // A bar's floating commit action; tested in
                      // SettingsHeaderButton.test.
                      { SettingsHeaderButton: "SettingsHeaderButton" }
                  : name.endsWith("/i18n/Translation")
                    ? richTranslationMock
                  : name.endsWith("/LanguageProvider")
                  ? {
                      useAppLanguage: () => ({
                        language: "en",
                        t: englishTranslation,
                      }),
                    }
                  : require(name),
          Error: realmError,
          AbortController,
          URL,
          URLSearchParams,
          queueMicrotask,
          requestAnimationFrame: (callback: () => void) =>
            setTimeout(callback, 0),
          cancelAnimationFrame: clearTimeout,
          setTimeout,
          clearTimeout,
          // Polling is not advanced by these lifecycle/terminal-refresh tests.
          setInterval: () => 1,
          clearInterval: () => {},
          ...globals,
        },
        { filename },
      );
      return exports;
    },
    mount: (render: () => T) => {
      renderHook = render;
      dirty = true;
      return flush();
    },
    render: () => {
      dirty = true;
      return flush();
    },
    flush,
    value: () => value,
    settle: async () => {
      for (let i = 0; i < 5; i++) {
        await new Promise<void>((resolve) => setImmediate(resolve));
        flush();
      }
      return value;
    },
    unmount: () => {
      for (const effect of effects.values()) effect.cleanup?.();
      effects.clear();
    },
  };
  return harness;
}
