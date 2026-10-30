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

export function hookHarness<T>(mocks: Record<string, unknown>) {
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
  return {
    load: (filename: string) => {
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
                : require(name),
          AbortController,
          URL,
          URLSearchParams,
          queueMicrotask,
          requestAnimationFrame: (callback: () => void) => setTimeout(callback, 0),
          cancelAnimationFrame: clearTimeout,
          setTimeout,
          clearTimeout,
          // Polling is not advanced by these lifecycle/terminal-refresh tests.
          setInterval: () => 1,
          clearInterval: () => {},
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
}
