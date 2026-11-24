import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { createMemoryHistory, createRouter, RouterProvider } from "@tanstack/react-router";
import { act, render, screen } from "@testing-library/react";
import { expect, it } from "vitest";
import { isPublicPath } from "@/lib/public-paths";
import { router as appRouter } from "@/router";

it("opens the public Activity fallback with a fixed native destination", async () => {
  const history = createMemoryHistory({
    initialEntries: ["/activity?redirect=https%3A%2F%2Fexample.com&challengeId=untrusted"],
  });
  const router = createRouter({ routeTree: appRouter.routeTree, history });
  render(
    <QueryClientProvider client={new QueryClient()}>
      <RouterProvider router={router} />
    </QueryClientProvider>,
  );
  await act(() => router.load());

  expect(isPublicPath("/activity")).toBe(true);
  expect(isPublicPath("/activity/")).toBe(true);
  expect(isPublicPath("/activity/private")).toBe(false);
  expect(await screen.findByRole("heading", { name: "Activity" })).toBeVisible();
  expect(screen.getByRole("link", { name: "Open NyxID app" })).toHaveAttribute(
    "href", "nyxid://activity",
  );
  expect(screen.getByRole("link", { name: "Review approvals on the web" })).toHaveAttribute(
    "href", "/assistant/approvals",
  );
});
