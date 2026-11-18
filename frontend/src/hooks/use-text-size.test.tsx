import { act, renderHook, waitFor } from "@testing-library/react";
import { afterEach, expect, it } from "vitest";
import { useTextSize } from "./use-text-size";

afterEach(() => document.documentElement.removeAttribute("style"));

it("updates numeric chart geometry when Display changes the root without a window resize", async () => {
  const root = document.documentElement;
  root.style.setProperty("--text-13", "0.8125rem");
  root.style.fontSize = "13.6px";
  const view = renderHook(() => useTextSize(13));
  expect(view.result.current).toBeCloseTo(11.05);
  act(() => { root.style.fontSize = "27.2px"; });
  await waitFor(() => expect(view.result.current).toBeCloseTo(22.1));
  expect(root.style.fontSize).toBe("27.2px");
  view.unmount();
});
