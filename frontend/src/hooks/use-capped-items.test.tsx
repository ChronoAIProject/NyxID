import type { ReactNode } from "react";
import { act, renderHook } from "@testing-library/react";
import { expect, it } from "vitest";
import { DrawerSearchContext } from "@/lib/drawer-search";
import { useCappedItems } from "./use-capped-items";

it("caps growing lists without changing their order or dropping newly loaded items after Show all", () => {
  const { result, rerender } = renderHook(({ rows }) => useCappedItems(rows, 3), {
    initialProps: { rows: ["a", "b", "c", "d", "e"] },
  });
  expect(result.current.items).toEqual(["a", "b", "c"]);
  expect(result.current.remaining).toBe(2);
  act(() => result.current.showAll());
  rerender({ rows: ["a", "b", "c", "d", "e", "f"] });
  expect(result.current.items).toEqual(["a", "b", "c", "d", "e", "f"]);
  expect(result.current.remaining).toBe(0);
});

it("makes every loaded item searchable and retains the revealed item when search clears", () => {
  let searching = false;
  const noop = () => {};
  const wrapper = ({ children }: { children: ReactNode }) => (
    <DrawerSearchContext value={{ register: noop, remove: noop, registerPage: noop, searching }}>
      {children}
    </DrawerSearchContext>
  );
  const { result, rerender } = renderHook(() => useCappedItems(["first", "hidden"], 1), { wrapper });
  expect(result.current.items).toEqual(["first"]);
  searching = true;
  rerender();
  expect(result.current.items).toContain("hidden");
  searching = false;
  rerender();
  expect(result.current.items).toContain("hidden");
  expect(result.current.remaining).toBe(0);
});
