import { useState } from "react";
import { act, cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { createBrowserHistory, createMemoryHistory, useRouter } from "@tanstack/react-router";
import { afterEach, expect, it, vi } from "vitest";
import { GroupedDraftBoundary, GroupedDraftDialog, GroupedDraftNavigation } from "./grouped-drafts";
import { useGroupedDraft } from "@/lib/grouped-drafts";

vi.mock("@tanstack/react-router", async (original) => ({ ...await original<typeof import("@tanstack/react-router")>(), useRouter: vi.fn() }));
afterEach(cleanup);

it("guards URL Back before the sheet unmounts and permits it only after Discard", async () => {
  window.history.replaceState(null, "", "/assistant");
  const history = createBrowserHistory();
  history.push("/assistant?agent=researcher&details=grants");
  history.flush();
  vi.mocked(useRouter).mockReturnValue({ history } as ReturnType<typeof useRouter>);
  function Sheet() {
    const [dirty, setDirty] = useState(true);
    useGroupedDraft({ dirty, discard: () => setDirty(false) });
    return <><GroupedDraftNavigation active /><GroupedDraftDialog /><p>{dirty ? "Draft retained" : "Draft discarded"}</p></>;
  }
  render(<GroupedDraftBoundary><Sheet /></GroupedDraftBoundary>);
  await act(async () => history.back());
  expect(await screen.findByRole("dialog", { name: "Discard unsaved changes?" })).toBeInTheDocument();
  expect(history.location.search).toContain("details=grants");
  await userEvent.click(screen.getByRole("button", { name: "Keep editing" }));
  expect(screen.getByText("Draft retained")).toBeInTheDocument();
  expect(history.location.search).toContain("details=grants");
  await act(async () => history.back());
  await userEvent.click(screen.getByRole("button", { name: "Discard" }));
  await waitFor(() => expect(history.location.search).toBe(""));
  expect(screen.getByText("Draft discarded")).toBeInTheDocument();
  history.destroy();
});

it("retains dirty drafts without prompting during the same drawer's latest-thread replacement", async () => {
  const history = createMemoryHistory({ initialEntries: ["/assistant?agent=researcher&details=overview"] });
  vi.mocked(useRouter).mockReturnValue({ history } as ReturnType<typeof useRouter>);
  function Sheet() {
    const [dirty, setDirty] = useState(true);
    useGroupedDraft({ dirty, discard: () => setDirty(false) });
    return <><GroupedDraftNavigation active /><GroupedDraftDialog /><p>{dirty ? "Draft retained" : "Draft discarded"}</p></>;
  }
  render(<GroupedDraftBoundary><Sheet /></GroupedDraftBoundary>);
  await act(async () => {
    history.replace("/assistant?c=latest&agent=researcher&details=overview");
  });
  await waitFor(() => expect(history.location.search).toContain("c=latest"));
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  expect(screen.getByText("Draft retained")).toBeInTheDocument();
  // Replacing the URL to close the drawer still requires the dirty guard.
  await act(async () => history.replace("/assistant?c=latest"));
  expect(await screen.findByRole("dialog", { name: "Discard unsaved changes?" })).toBeInTheDocument();
  await userEvent.click(screen.getByRole("button", { name: "Keep editing" }));
  history.destroy();
});

it.each([
  ["/settings?tab=profile", "/settings?tab=security", "/keys"],
  ["/assistant?panel=settings&panelTab=profile", "/assistant?panel=settings&panelTab=security&c=adopted", "/assistant?c=adopted"],
])("retains settings drafts across tab replacement and guards leaving %s", async (initial, tab, leave) => {
  const history = createMemoryHistory({ initialEntries: [initial!] });
  vi.mocked(useRouter).mockReturnValue({ history } as ReturnType<typeof useRouter>);
  function Settings() {
    const [dirty, setDirty] = useState(true);
    useGroupedDraft({ dirty, discard: () => setDirty(false) });
    return <><GroupedDraftNavigation active /><GroupedDraftDialog /><p>{dirty ? "Draft retained" : "Draft discarded"}</p></>;
  }
  render(<GroupedDraftBoundary><Settings /></GroupedDraftBoundary>);
  await act(async () => history.replace(tab!));
  await waitFor(() => expect(history.location.href).toBe(tab));
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  expect(screen.getByText("Draft retained")).toBeInTheDocument();
  await act(async () => history.replace(leave!));
  expect(await screen.findByRole("dialog", { name: "Discard unsaved changes?" })).toBeInTheDocument();
  await userEvent.click(screen.getByRole("button", { name: "Discard" }));
  await waitFor(() => expect(history.location.href).toBe(leave));
  expect(screen.getByText("Draft discarded")).toBeInTheDocument();
  history.destroy();
});
