import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { createMemoryHistory, createRootRoute, createRoute, createRouter, RouterProvider, useNavigate, useRouterState } from "@tanstack/react-router";
import { expect, it } from "vitest";
import { parseAssistantSearch } from "@/lib/assistant/search";
import { useAgentDetailsNavigation } from "./use-agent-details-navigation";

function SheetNavigation() {
  const { open, close } = useAgentDetailsNavigation();
  const navigate = useNavigate();
  const search = useRouterState({ select: state => state.location.search });
  return <>
    <button onClick={() => open("specialist")}>Open local sheet</button>
    <button onClick={() => void navigate({ to: "/assistant", search: previous => ({ ...previous, agent: "specialist", details: "overview" }) })}>Open linked sheet</button>
    <button onClick={() => close()}>Close sheet</button>
    {(["grants", "overview/persona", "overview"] as const).map(details => <button key={details} onClick={() => void navigate({ to: "/assistant", search: previous => ({ ...previous, details }), state: previous => previous, replace: true })}>{details}</button>)}
    <output>{search.details ?? "closed"}</output>
  </>;
}

async function setup() {
  const root = createRootRoute();
  const route = createRoute({ getParentRoute: () => root, path: "/assistant", validateSearch: parseAssistantSearch, component: SheetNavigation });
  const history = createMemoryHistory({ initialEntries: ["/assistant?c=thread&agent=nyxbot&mock=1"] });
  const router = createRouter({ routeTree: root.addChildren([route]), history });
  render(<RouterProvider router={router} />);
  await act(() => router.load());
  return { router, history, user: userEvent.setup() };
}

it("keeps formerly local sheet opens, tabs, sections and closes in one entry", async () => {
  const { router, history, user } = await setup();
  const original = router.state.location.search;
  await user.click(screen.getByText("Open local sheet"));
  expect(router.state.location.search).toMatchObject({ c: "thread", agent: "specialist", details: "overview", mock: 1 });
  expect(history.length).toBe(1);
  for (const value of ["grants", "overview/persona", "overview"]) {
    await user.click(screen.getByRole("button", { name: value }));
    expect(history.length).toBe(1);
    expect(router.state.location.search.details).toBe(value);
  }
  await user.click(screen.getByText("Close sheet"));
  expect(history.length).toBe(1);
  expect(router.state.location.search).toEqual(original);
  expect(history.location.state.agentDetailsOrigin).toBeUndefined();
});

it("retains push opening/closing for linked sheets and replace for tabs and sections", async () => {
  const { router, history, user } = await setup();
  await user.click(screen.getByText("Open linked sheet"));
  expect(history.length).toBe(2);
  await user.click(screen.getByRole("button", { name: "grants" }));
  await user.click(screen.getByRole("button", { name: "overview/persona" }));
  expect(history.length).toBe(2);
  await act(() => history.back());
  await waitFor(() => expect(router.state.location.search.details).toBeUndefined());
  await act(() => history.forward());
  await waitFor(() => expect(router.state.location.search.details).toBe("overview/persona"));
  await user.click(screen.getByText("Close sheet"));
  expect(history.length).toBe(3);
  await act(() => history.back());
  await waitFor(() => expect(router.state.location.search.details).toBe("overview/persona"));
});
