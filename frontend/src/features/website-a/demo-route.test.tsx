import { act, cleanup, render, screen } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import { WebsitePage } from "./website-page";
import { useWebsite } from "./website-context";
const location = vi.hoisted(() => ({
  pathname: "/nyxbot",
  searchStr: "",
  hash: "",
}));
vi.mock("@tanstack/react-router", () => ({ useLocation: () => location }));
vi.mock("./generated/en/WebsiteHeader", () => ({ default: () => null }));
vi.mock("./generated/zh/WebsiteHeader", () => ({ default: () => null }));
vi.mock("./generated/en/WebsiteFooter", () => ({ default: () => null }));
vi.mock("./generated/zh/WebsiteFooter", () => ({ default: () => null }));
function DemoIndex() {
  return <output data-testid="demo-index">{useWebsite().demoIndex}</output>;
}
afterEach(() => {
  cleanup();
  vi.useRealTimers();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});
it.each(["/nyxbot", "/zh/nyxbot"])(
  "advances the approved demo for %s without reduced motion",
  (path) => {
    vi.useFakeTimers();
    location.pathname = path;
    vi.stubGlobal("matchMedia", () => ({
      matches: false,
      addEventListener() {},
      removeEventListener() {},
    }));
    vi.spyOn(document, "hidden", "get").mockReturnValue(false);
    render(
      <WebsitePage>
        <DemoIndex />
      </WebsitePage>,
    );
    expect(screen.getByTestId("demo-index")).toHaveTextContent("0");
    act(() => vi.advanceTimersByTime(7000));
    expect(screen.getByTestId("demo-index")).toHaveTextContent("1");
  },
);
