import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import {
  createMemoryHistory,
  createRootRoute,
  createRoute,
  createRouter,
  RouterProvider,
} from "@tanstack/react-router";
import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { apiClient } from "@/lib/api-client";
import { nyxbotI18n } from "@/features/nyxbot-onboarding/i18n";
import { parseAssistantSearch } from "@/lib/assistant/search";
import { NyxbotStartPage } from "./nyxbot-start";
import { parseNyxbotStartSearch } from "@/lib/assistant/business-starters";
vi.mock("@/lib/api-client", () => ({ apiClient: vi.fn() }));
const cases = [
  {
    id: "queue-manager",
    title: "Queue manager",
    description: "Manage your waiting list",
    example_prompts: ["Set up queue"],
  },
  {
    id: "business-faq",
    title: "Business FAQ",
    description: "Answer business questions",
    example_prompts: ["Set up FAQ"],
  },
];
beforeEach(() => {
  vi.mocked(apiClient).mockResolvedValue({ use_cases: cases });
  void nyxbotI18n.changeLanguage("en");
});
afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});
async function open(path = "/nyxbot/start") {
  const root = createRootRoute();
  const start = createRoute({
    getParentRoute: () => root,
    path: "/nyxbot/start",
    validateSearch: parseNyxbotStartSearch,
    component: NyxbotStartPage,
  });
  const assistant = createRoute({
    getParentRoute: () => root,
    path: "/assistant",
    validateSearch: parseAssistantSearch,
    component: () => <div>Assistant destination</div>,
  });
  const router = createRouter({
    routeTree: root.addChildren([start, assistant]),
    history: createMemoryHistory({ initialEntries: [path] }),
  });
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  await router.load();
  render(
    <QueryClientProvider client={client}>
      <RouterProvider router={router} />
    </QueryClientProvider>,
  );
  return router;
}
it("selects published use cases and navigates to a new NyxBot draft", async () => {
  const router = await open();
  const button = await screen.findByRole("button", {
    name: "Continue in NyxBot",
  });
  expect(button).toBeDisabled();
  fireEvent.click(
    await screen.findByRole("checkbox", { name: "Queue manager" }),
  );
  fireEvent.click(screen.getByRole("checkbox", { name: "Business FAQ" }));
  expect(screen.getByText("Manage your waiting list")).toBeInTheDocument();
  fireEvent.click(button);
  await waitFor(() =>
    expect(router.state.location.pathname).toBe("/assistant"),
  );
  expect(router.state.location.search).toEqual({
    draft: true,
    starter: "queue-manager,business-faq",
  });
  expect(apiClient).toHaveBeenCalledWith(
    "/public/help/use-cases",
    expect.objectContaining({ credentials: "omit" }),
  );
});
it("preselects valid unique ids and ignores unknown selections", async () => {
  await open("/nyxbot/start?usecases=queue-manager,queue-manager,unknown,Bad");
  expect(
    await screen.findByRole("checkbox", { name: "Queue manager" }),
  ).toBeChecked();
  expect(
    screen.getByRole("checkbox", { name: "Business FAQ" }),
  ).not.toBeChecked();
  fireEvent.click(screen.getByRole("checkbox", { name: "Queue manager" }));
  expect(
    screen.getByRole("button", { name: "Continue in NyxBot" }),
  ).toBeDisabled();
});
it("shows a loading state", async () => {
  vi.mocked(apiClient).mockReturnValue(new Promise(() => {}));
  await open();
  expect(screen.getByRole("status")).toBeInTheDocument();
});
it("shows the empty publication state", async () => {
  vi.mocked(apiClient).mockResolvedValue({ use_cases: [] });
  await open();
  expect(
    await screen.findByText("No use cases are available yet."),
  ).toBeInTheDocument();
  expect(
    screen.getByRole("button", { name: "Continue in NyxBot" }),
  ).toBeDisabled();
});
it("shows an error and retries", async () => {
  vi.mocked(apiClient).mockRejectedValueOnce(new Error("offline"));
  await open();
  expect(await screen.findByRole("alert")).toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "Retry" }));
  expect(
    await screen.findByRole("checkbox", { name: "Queue manager" }),
  ).toBeInTheDocument();
});
it("uses the same bounded preselection contract", () => {
  expect(
    parseNyxbotStartSearch({ usecases: "a,a,Bad,b,c,d,e,f,g,h,i" }),
  ).toEqual({ usecases: "a,b,c,d,e,f,g,h" });
});
