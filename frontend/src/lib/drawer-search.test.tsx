import { useState } from "react";
import { cleanup, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, expect, it } from "vitest";
import {
  DrawerSearchProvider,
  AgentDrawerSearch,
} from "@/components/assistant/agent-drawer-search";
import {
  DrawerSearchTabContext,
  drawerMatchRanges,
  matchDrawerEntries,
  normalizeDrawerSearch,
  useDrawerSearchEntries,
  type DrawerSearchEntry,
} from "./drawer-search";

afterEach(cleanup);
const entry = (
  id: string,
  title: string,
  keywords = "",
): DrawerSearchEntry => ({ id, title, keywords, tab: "overview" });
it("matches case and diacritics with all query words across live metadata", () => {
  expect(normalizeDrawerSearch("CRÈME Café")).toBe("creme cafe");
  const values = [
    entry("a", "Café bot", "Telegram connected"),
    entry("b", "Cafe research", "Slack"),
  ];
  expect(
    matchDrawerEntries(values, "CAFÉ tele").map((value) => value.id),
  ).toEqual(["a"]);
  expect(matchDrawerEntries(values, "cafe missing")).toEqual([]);
  expect(matchDrawerEntries(values, "  ")).toEqual([]);
});
it("ranks exact titles, prefixes, title matches, then metadata and caps deterministically", () => {
  const values = [
    entry("metadata", "A service", "model"),
    entry("contains", "Your model"),
    entry("prefix", "Model access"),
    entry("exact", "Model"),
  ];
  expect(matchDrawerEntries(values, "model").map((value) => value.id)).toEqual([
    "exact",
    "prefix",
    "contains",
    "metadata",
  ]);
  expect(
    matchDrawerEntries(values, "model", 2).map((value) => value.id),
  ).toEqual(["exact", "prefix"]);
  expect(
    matchDrawerEntries([entry("z", "same"), entry("a", "same")], "same").map(
      (value) => value.id,
    ),
  ).toEqual(["a", "z"]);
});
it("maps accent, combining-mark and surrogate offsets to original highlighted text", () => {
  expect(drawerMatchRanges("☕ Café cafe\u0301", "cafe")).toEqual([
    [2, 6],
    [7, 12],
  ]);
  expect(drawerMatchRanges("😀 café", "café")).toEqual([[3, 7]]);
  expect(drawerMatchRanges("model model", "mod model")).toEqual([
    [0, 5],
    [6, 11],
  ]);
});
function Entries({ title }: { readonly title: string }) {
  useDrawerSearchEntries([{ id: "live", title }]);
  return null;
}
function Registry() {
  const [query, setQuery] = useState("fresh");
  const [title, setTitle] = useState("Old title");
  const [show, setShow] = useState(true);
  return (
    <DrawerSearchProvider query={query}>
      <button onClick={() => setTitle("Fresh title")}>Update data</button>
      <button onClick={() => setShow(false)}>Remove section</button>
      <DrawerSearchTabContext value="overview">
        {show && <Entries title={title} />}
      </DrawerSearchTabContext>
      <AgentDrawerSearch
        query={query}
        setQuery={setQuery}
        selected="overview"
        selectTab={() => {}}
        container={null}
      />
    </DrawerSearchProvider>
  );
}
it("updates the registry with section data and removes entries when a section unmounts", async () => {
  const user = userEvent.setup();
  render(<Registry />);
  expect(screen.getByText("No results for “fresh”.")).toBeInTheDocument();
  await user.click(screen.getByRole("button", { name: "Update data" }));
  expect(
    screen.getByRole("button", { name: "Fresh title Overview" }),
  ).toBeInTheDocument();
  await user.click(screen.getByRole("button", { name: "Remove section" }));
  expect(screen.getByText("No results for “fresh”.")).toBeInTheDocument();
});
