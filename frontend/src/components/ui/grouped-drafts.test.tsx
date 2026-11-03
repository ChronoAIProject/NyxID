import { useState } from "react";
import { act, cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { createBrowserHistory, useRouter } from "@tanstack/react-router";
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
