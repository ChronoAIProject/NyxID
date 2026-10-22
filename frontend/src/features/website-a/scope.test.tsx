import { render, screen, cleanup } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import { SolutionActivation } from "./solution-activation";
import { legacyDestination } from "./auth-links";
import { publishedPages } from "../public-site/publication";
vi.mock("./website-context", () => ({
  useWebsite: () => ({ language: "en" }),
}));
afterEach(cleanup);
it("exposes native disabled activation without a link or handler", () => {
  const { container } = render(<SolutionActivation />);
  expect(screen.getByRole("button")).toBeDisabled();
  expect(container.querySelector("a")).toBeNull();
  expect(screen.getByText("Not available yet")).toBeVisible();
});
it("does not publish candidate articles or let legacy solution activation skip the gate", () => {
  expect(publishedPages.some((p) => p.route.startsWith("/blog/"))).toBe(false);
  expect(legacyDestination("#/start?t=help-desk", "?code=INV")).toBe(
    "/solutions/help-desk?code=INV",
  );
  expect(legacyDestination("#/start?t=unknown", "")).toBe("/solutions");
});
