import { useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, afterEach, describe, expect, it, vi } from "vitest";
import { AsyncOptionSelect } from "./async-option-select";
import { ServiceAccountScopePicker } from "@/components/service-accounts/service-account-scope-picker";
import { Form, FormField, FormItem, FormControl, FormLabel, useAppForm } from "@/components/ui/form";
import { Dialog, DialogContent, DialogTitle } from "@/components/ui/dialog";
import { useAuthStore } from "@/stores/auth-store";
import { optionsResponse, optionsWrapper } from "@/test-utils/options";
import type { User } from "@/types/api";

const fetchMock = vi.fn<typeof fetch>();
beforeEach(() => {
  fetchMock.mockReset();
  vi.stubGlobal("fetch", fetchMock);
  useAuthStore.setState({ user: { id: "actor" } as User });
  fetchMock.mockImplementation(async (url) => new Response(JSON.stringify(optionsResponse(String(url)))));
});
afterEach(() => { vi.unstubAllGlobals(); vi.clearAllMocks(); });

function Picker({ owner = "owner", initial = [], custom = false }: { owner?: string; initial?: string[]; custom?: boolean }) {
  const [values, setValues] = useState(initial);
  return <><AsyncOptionSelect optionSet="service-scope" context={{ owner_id: owner, kind: "service-scope", principal_type: "service_account" }} label="Scopes" value={values} onChange={setValues} allowCustom={custom} delimiter=":" /><output aria-label="Selected scopes">{values.join(" ")}</output></>;
}

function nestedResponse(url: string) {
  const data = optionsResponse(url);
  data.items = ["reports:read", "reports:export", "reports:finance:read", "reports:finance:export", "metrics:read", "llm:proxy", "roles", "proxy"].map((value) => ({ ...data.items[0]!, value, label: value, source: "configured_scope" }));
  return data;
}

describe("editable async options selection", () => {
  it("selects both catalog skill scopes from options without submitting navigation prefixes", async () => {
    fetchMock.mockImplementation(async (url) => {
      const data = optionsResponse(String(url));
      const search = new URL(String(url), "http://localhost").searchParams.get("search") ?? "";
      data.items = ["catalog:skills:read", "catalog:skills:write"].map((value) => ({
        ...data.items[0]!, value, label: value.endsWith("read") ? "Read catalog skills" : "Manage catalog skills",
      })).filter((item) => item.value.includes(search));
      data.total = data.items.length;
      return new Response(JSON.stringify(data));
    });
    function Editor() {
      const [scopes, setScopes] = useState("");
      return <><ServiceAccountScopePicker ownerId="owner" value={scopes} onChange={setScopes} /><output aria-label="Stored scopes">{scopes}</output></>;
    }
    const user = userEvent.setup();
    render(<Editor />, { wrapper: optionsWrapper() });
    await user.click(screen.getByRole("combobox", { name: "Allowed scopes" }));
    for (const action of ["read", "write"]) {
      await user.click(await screen.findByRole("option", { name: "Explore catalog:" }));
      await user.click(await screen.findByRole("option", { name: "Explore catalog:skills:" }));
      expect(screen.getByRole("status", { name: "Stored scopes" })).toHaveTextContent(action === "read" ? /^$/ : /^catalog:skills:read$/);
      await user.click(await screen.findByRole("option", { name: `catalog:skills:${action}` }));
    }
    expect(screen.getByRole("status", { name: "Stored scopes" })).toHaveTextContent(/^catalog:skills:read catalog:skills:write$/);
    await user.click(screen.getByRole("button", { name: "Remove catalog:skills:read" }));
    expect(screen.getByRole("status", { name: "Stored scopes" })).toHaveTextContent(/^catalog:skills:write$/);
  });

  it("selects multiple suggestions and retains raw chips during remote search", async () => {
    const user = userEvent.setup();
    render(<Picker />, { wrapper: optionsWrapper() });
    const input = screen.getByRole("combobox", { name: "Scopes" });
    await user.click(input);
    await user.click(await screen.findByRole("option", { name: "proxy" }));
    await user.click(screen.getByRole("option", { name: "roles" }));
    expect(screen.getByRole("status", { name: "Selected scopes" })).toHaveTextContent("proxy roles");
    await user.type(input, "missing");
    await waitFor(() => expect(fetchMock.mock.calls.some(([url]) => String(url).includes("search=missing"))).toBe(true));
    expect(screen.getByRole("button", { name: "Remove proxy" })).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Remove proxy" }));
    expect(screen.getByRole("status", { name: "Selected scopes" })).toHaveTextContent("roles");
  });

  it("shows errors in the dropdown and retries without fallback suggestions", async () => {
    fetchMock.mockImplementation(async () => new Response(JSON.stringify({ message: "Unavailable", error_code: 1000 }), { status: 503 }));
    render(<Picker />, { wrapper: optionsWrapper() });
    await userEvent.click(screen.getByRole("combobox"));
    expect(await screen.findByRole("alert")).toHaveTextContent("Suggestions unavailable");
    expect(screen.queryByRole("option")).not.toBeInTheDocument();
    fetchMock.mockImplementation(async (url) => new Response(JSON.stringify(optionsResponse(String(url)))));
    await userEvent.click(screen.getByRole("button", { name: "Retry suggestions" }));
    expect(await screen.findByRole("option", { name: "proxy" })).toBeInTheDocument();
  });

  it("isolates query data by owner and identity while retaining custom selections", async () => {
    fetchMock.mockImplementation(async (url) => {
      const data = optionsResponse(String(url));
      data.items[0]!.label = `${data.owner_id} services`;
      return new Response(JSON.stringify(data));
    });
    const view = render(<Picker owner="first" initial={["legacy:read"]} />, { wrapper: optionsWrapper() });
    await userEvent.click(screen.getByRole("combobox"));
    expect(await screen.findByText("first services")).toBeInTheDocument();
    view.rerender(<Picker owner="second" initial={["legacy:read"]} />);
    await userEvent.click(screen.getByRole("combobox"));
    expect(screen.queryByText("first services")).not.toBeInTheDocument();
    expect(await screen.findByText("second services")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Remove legacy:read" })).toBeInTheDocument();
    const before = fetchMock.mock.calls.length;
    act(() => useAuthStore.setState({ user: { id: "another-actor" } as User }));
    await waitFor(() => expect(fetchMock.mock.calls.length).toBeGreaterThan(before));
  });

  it("preserves duplicate stored values until an edit and marks form dirty", async () => {
    function Editor() {
      const form = useAppForm({ defaultValues: { scopes: "legacy:read legacy:read" } });
      return <Form {...form}><FormField control={form.control} name="scopes" render={({ field }) => <FormItem><FormLabel>Allowed Scopes</FormLabel><FormControl><ServiceAccountScopePicker ownerId="owner" serviceAccountId="account" {...field} /></FormControl></FormItem>} /><button disabled={!form.formState.isDirty}>Save</button><output aria-label="Stored scopes">{form.watch("scopes")}</output></Form>;
    }
    render(<Editor />, { wrapper: optionsWrapper() });
    expect(screen.getByRole("button", { name: "Save" })).toBeDisabled();
    expect(screen.getAllByRole("button", { name: "Remove legacy:read" })).toHaveLength(1);
    expect(screen.getByRole("status", { name: "Stored scopes" })).toHaveTextContent("legacy:read legacy:read");
    await userEvent.click(screen.getByRole("combobox"));
    await userEvent.click(await screen.findByRole("option", { name: "roles" }));
    expect(screen.getByRole("button", { name: "Save" })).toBeEnabled();
    await userEvent.click(screen.getByRole("button", { name: "Remove legacy:read" }));
    expect(screen.getByRole("status", { name: "Stored scopes" })).toHaveTextContent(/^roles$/);
  });

  it.each(["personal", "organization"])("preserves typing and focus while the initial identity loads for a %s owner", async (ownerKind) => {
    useAuthStore.setState({ user: null });
    fetchMock.mockImplementation(async (url) => new Response(JSON.stringify(nestedResponse(String(url)))));
    function LoadingPicker() {
      const identity = useAuthStore((state) => state.user?.id);
      return <Picker custom owner={ownerKind === "personal" ? identity ?? "" : "organization"} />;
    }
    const user = userEvent.setup();
    render(<LoadingPicker />, { wrapper: optionsWrapper() });
    const input = screen.getByRole("combobox");
    await user.type(input, "reports");
    expect(fetchMock).not.toHaveBeenCalled();
    act(() => useAuthStore.setState({ user: { id: "actor" } as User }));
    expect(screen.getByRole("combobox")).toBe(input);
    expect(input).toHaveValue("reports");
    expect(input).toHaveFocus();
    await user.click(await screen.findByRole("option", { name: "Explore reports:" }));
    await user.click(screen.getByRole("option", { name: "reports:read" }));
    expect(screen.getByRole("status", { name: "Selected scopes" })).toHaveTextContent(/^reports:read$/);
  });

  it("restarts paging when response versions change", async () => {
    let initialRequests = 0;
    fetchMock.mockImplementation(async (url) => {
      const offset = new URL(String(url), "http://localhost").searchParams.get("offset");
      if (offset === "0") initialRequests += 1;
      const changed = offset !== "0" || initialRequests > 1;
      const data = optionsResponse(String(url), { version: changed ? "new" : "old", total: 150, next_offset: changed ? null : 50 });
      data.items = [{ ...data.items[0]!, label: changed ? "Fresh choice" : "Old choice" }];
      return new Response(JSON.stringify(data));
    });
    render(<Picker />, { wrapper: optionsWrapper() });
    await userEvent.click(screen.getByRole("combobox"));
    expect(await screen.findByText("Fresh choice")).toBeInTheDocument();
    expect(screen.queryByText("Old choice")).not.toBeInTheDocument();
    expect(initialRequests).toBe(2);
  });

  it("walks only real colon prefixes, fetches each prefix, and adds only a complete leaf", async () => {
    fetchMock.mockImplementation(async (url) => new Response(JSON.stringify(nestedResponse(String(url)))));
    const user = userEvent.setup();
    render(<Picker custom />, { wrapper: optionsWrapper() });
    const input = screen.getByRole("combobox");
    await user.click(input);
    await user.click(await screen.findByRole("option", { name: "Explore reports:" }));
    expect(input).toHaveValue("reports:");
    expect(screen.getByRole("status", { name: "Selected scopes" })).toBeEmptyDOMElement();
    expect(screen.queryByRole("option", { name: "reports:proxy" })).not.toBeInTheDocument();
    await user.click(screen.getByRole("option", { name: "Explore reports:finance:" }));
    await waitFor(() => expect(fetchMock.mock.calls.some(([url]) => String(url).includes("search=reports%3Afinance%3A"))).toBe(true));
    expect(screen.queryByRole("option", { name: "reports:finance:proxy" })).not.toBeInTheDocument();
    await user.click(await screen.findByRole("option", { name: "reports:finance:export" }));
    expect(screen.getByRole("button", { name: "Remove reports:finance:export" })).toBeInTheDocument();
    expect(screen.getByRole("status", { name: "Selected scopes" })).toHaveTextContent(/^reports:finance:export$/);
  });

  it("adds exact typed values unless arrows explicitly select a suggestion and shows every selected pill", async () => {
    fetchMock.mockImplementation(async (url) => new Response(JSON.stringify(nestedResponse(String(url)))));
    const user = userEvent.setup();
    render(<Picker custom />, { wrapper: optionsWrapper() });
    const input = screen.getByRole("combobox");
    await user.type(input, "reports{Enter}");
    expect(screen.getByRole("button", { name: "Remove reports" })).toBeInTheDocument();
    await user.type(input, "reports");
    await screen.findByRole("option", { name: "Explore reports:" });
    await user.keyboard("{ArrowDown}{Enter}");
    expect(input).toHaveValue("reports:");
    await user.clear(input);
    await user.paste("custom:one custom:two\ncustom:one custom:three");
    for (const value of ["reports", "custom:one", "custom:two", "custom:three"]) expect(screen.getByRole("button", { name: `Edit ${value}` })).toBeVisible();
    await user.click(screen.getByRole("button", { name: "Remove custom:three" }));
    expect(screen.getByRole("status", { name: "Selected scopes" })).toHaveTextContent(/^reports custom:one custom:two$/);
  });

  it.each(["error", "empty"])("submits custom typing and paste while suggestions are %s", async (state) => {
    fetchMock.mockImplementation(async (url) => state === "error" ? new Response(JSON.stringify({ message: "Unavailable" }), { status: 503 }) : new Response(JSON.stringify(optionsResponse(String(url), { items: [], total: 0 }))));
    const saved = vi.fn();
    function Editor() {
      const form = useAppForm({ defaultValues: { scopes: "existing:read" } });
      return <form onSubmit={form.handleSubmit(saved)}><ServiceAccountScopePicker ownerId="owner" value={form.watch("scopes")} onChange={(v) => form.setValue("scopes", v)} /><button>Save custom scopes</button></form>;
    }
    const user = userEvent.setup();
    render(<Editor />, { wrapper: optionsWrapper() });
    const input = screen.getByRole("combobox");
    await user.click(input);
    if (state === "error") await screen.findByRole("alert"); else await screen.findByText("No matching suggestions.");
    await user.type(input, "custom:read{Enter}");
    await user.paste("custom:write custom:read");
    await user.type(input, "draft:scope");
    await user.click(screen.getByRole("button", { name: "Save custom scopes" }));
    await waitFor(() => expect(saved).toHaveBeenCalled());
    expect(saved.mock.calls[0]![0]).toEqual({ scopes: "existing:read custom:read custom:write draft:scope" });
  });

  it("handles IME, Escape inside a modal, and keyboard blur before dirty-gated save", async () => {
    const saved = vi.fn();
    function Editor() {
      const form = useAppForm({ defaultValues: { scopes: "roles" } });
      return <Dialog open><DialogContent><DialogTitle>Edit scopes</DialogTitle><form onSubmit={form.handleSubmit(saved)}><ServiceAccountScopePicker ownerId="owner" value={form.watch("scopes")} onChange={(v) => form.setValue("scopes", v)} /><button disabled={!form.formState.isDirty}>Save draft</button></form></DialogContent></Dialog>;
    }
    const user = userEvent.setup();
    render(<Editor />, { wrapper: optionsWrapper() });
    const input = screen.getByRole("combobox");
    await user.type(input, "custom:draft");
    fireEvent.keyDown(input, { key: "Enter", isComposing: true });
    expect(input).toHaveValue("custom:draft");
    await user.keyboard("{Escape}");
    expect(screen.getByRole("dialog", { name: "Edit scopes" })).toBeInTheDocument();
    expect(input).toHaveAttribute("aria-expanded", "false");
    expect(screen.getByRole("button", { name: "Save draft" })).toBeEnabled();
    await user.tab();
    expect(screen.getByRole("button", { name: "Save draft" })).toBeEnabled();
    await user.click(screen.getByRole("button", { name: "Save draft" }));
    await waitFor(() => expect(saved).toHaveBeenCalled());
    expect(saved.mock.calls[0]![0]).toEqual({ scopes: "roles custom:draft" });
  });
  it("closes the suggestions on first Escape and the containing dialog on second Escape", async () => {
    function Editor() {
      const [open, setOpen] = useState(true);
      return <Dialog open={open} onOpenChange={setOpen}><DialogContent><DialogTitle>Edit scopes</DialogTitle><Picker custom /></DialogContent></Dialog>;
    }
    const user = userEvent.setup();
    render(<Editor />, { wrapper: optionsWrapper() });
    await user.click(screen.getByRole("combobox"));
    await screen.findByRole("option", { name: "proxy" });
    await user.keyboard("{Escape}");
    expect(screen.getByRole("dialog", { name: "Edit scopes" })).toBeInTheDocument();
    expect(screen.getByRole("combobox")).toHaveAttribute("aria-expanded", "false");
    await user.keyboard("{Escape}");
    expect(screen.queryByRole("dialog", { name: "Edit scopes" })).not.toBeInTheDocument();
  });

  it("starts ArrowUp at the last unselected choice and blocks changes when disabled while open", async () => {
    fetchMock.mockImplementation(async (url) => new Response(JSON.stringify(nestedResponse(String(url)))));
    const changed = vi.fn();
    const props = { optionSet: "service-scope" as const, context: { owner_id: "owner", kind: "service-scope" as const, principal_type: "service_account" as const }, label: "Scopes", value: ["roles"], onChange: changed, allowCustom: true };
    const user = userEvent.setup();
    const view = render(<AsyncOptionSelect {...props} />, { wrapper: optionsWrapper() });
    const input = screen.getByRole("combobox");
    await user.click(input);
    await screen.findByRole("option", { name: "proxy" });
    expect(screen.queryByRole("option", { name: "roles" })).not.toBeInTheDocument();
    await user.keyboard("{ArrowUp}{Enter}");
    expect(changed).toHaveBeenLastCalledWith(["roles", "proxy"]);
    changed.mockClear();
    await user.type(input, "proxy");
    changed.mockClear();
    view.rerender(<AsyncOptionSelect {...props} disabled />);
    fireEvent.click(screen.getByRole("option", { name: "proxy" }));
    fireEvent.blur(input, { relatedTarget: document.body });
    expect(changed).not.toHaveBeenCalled();
  });

  it("hides selected leaves and prunes branches only when every actual descendant is selected", async () => {
    fetchMock.mockImplementation(async (url) => new Response(JSON.stringify(nestedResponse(String(url)))));
    const user = userEvent.setup();
    render(<Picker custom initial={["reports:read", "reports:export", "reports:finance:read", "reports:finance:export", "roles"]} />, { wrapper: optionsWrapper() });
    await user.click(screen.getByRole("combobox"));
    await screen.findByRole("option", { name: "Explore metrics:" });
    expect(screen.queryByRole("option", { name: "Explore reports:" })).not.toBeInTheDocument();
    expect(screen.queryByRole("option", { name: "roles" })).not.toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Remove reports:finance:export" }));
    await user.click(screen.getByRole("combobox"));
    await user.click(screen.getByRole("option", { name: "Explore reports:" }));
    expect(screen.queryByRole("option", { name: "reports:read" })).not.toBeInTheDocument();
    await user.click(screen.getByRole("option", { name: "Explore reports:finance:" }));
    expect(screen.getAllByRole("option")).toHaveLength(1);
    expect(screen.getByRole("option", { name: "reports:finance:export" })).toBeInTheDocument();
  });

  it("loads every page automatically and keeps partial suggestions during a retryable page failure", async () => {
    let retry = false;
    fetchMock.mockImplementation(async (url) => {
      const offset = new URL(String(url), "http://localhost").searchParams.get("offset");
      if (offset === "100" && !retry) return new Response(JSON.stringify({ message: "Unavailable" }), { status: 503 });
      const data = optionsResponse(String(url), { total: 200, next_offset: offset === "0" ? 100 : null });
      data.items = [{ ...data.items[0]!, value: offset === "0" ? "first" : "last", label: offset === "0" ? "First page" : "Last page" }];
      return new Response(JSON.stringify(data));
    });
    const user = userEvent.setup();
    render(<Picker custom />, { wrapper: optionsWrapper() });
    await user.click(screen.getByRole("combobox"));
    expect(await screen.findByRole("alert")).toHaveTextContent("Some suggestions could not load");
    expect(screen.getByRole("option", { name: "first" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /Load more/ })).not.toBeInTheDocument();
    retry = true;
    await user.click(screen.getByRole("button", { name: "Retry suggestions" }));
    expect(await screen.findByRole("option", { name: "last" })).toBeInTheDocument();
    expect(screen.getByRole("option", { name: "first" })).toBeInTheDocument();
  });

  it("edits any pill in place, preserves ordering, cancels, and deduplicates replacements", async () => {
    const user = userEvent.setup();
    render(<Picker custom initial={["roles", "custom:old", "proxy"]} />, { wrapper: optionsWrapper() });
    await user.click(screen.getByRole("button", { name: "Edit custom:old" }));
    let input = screen.getByRole("combobox");
    expect(input).toHaveValue("custom:old");
    await user.clear(input);
    await user.type(input, "custom:new");
    expect(screen.getByRole("status", { name: "Selected scopes" })).toHaveTextContent(/^roles custom:new proxy$/);
    await user.keyboard("{Escape}");
    expect(screen.getByRole("status", { name: "Selected scopes" })).toHaveTextContent(/^roles custom:old proxy$/);
    expect(screen.getByRole("button", { name: "Edit custom:old" })).toHaveFocus();
    await user.keyboard("{Enter}");
    input = screen.getByRole("combobox");
    await user.clear(input);
    await user.type(input, "custom:new{Enter}");
    expect(screen.getByRole("status", { name: "Selected scopes" })).toHaveTextContent(/^roles custom:new proxy$/);
    await user.click(screen.getByRole("button", { name: "Edit custom:new" }));
    await user.clear(screen.getByRole("combobox"));
    await user.type(screen.getByRole("combobox"), "roles{Enter}");
    expect(screen.getByRole("status", { name: "Selected scopes" })).toHaveTextContent(/^roles proxy$/);
    expect(screen.getAllByRole("button", { name: "Edit roles" })).toHaveLength(1);
  });

  it("does not resurrect stale values when switching edits, removing another pill, or clearing an edit", async () => {
    const user = userEvent.setup();
    render(<Picker custom initial={["first", "second", "third"]} />, { wrapper: optionsWrapper() });
    await user.click(screen.getByRole("combobox"));
    await user.type(screen.getByRole("combobox"), "added");
    await user.click(screen.getByRole("button", { name: "Edit first" }));
    await user.clear(screen.getByRole("combobox"));
    await user.type(screen.getByRole("combobox"), "changed");
    await user.click(screen.getByRole("button", { name: "Edit second" }));
    expect(screen.getByRole("status", { name: "Selected scopes" })).toHaveTextContent(/^changed second third added$/);
    await user.clear(screen.getByRole("combobox"));
    await user.type(screen.getByRole("combobox"), "replacement");
    await user.click(screen.getByRole("button", { name: "Remove third" }));
    expect(screen.getByRole("status", { name: "Selected scopes" })).toHaveTextContent(/^changed replacement added$/);
    await user.click(screen.getByRole("button", { name: "Edit replacement" }));
    await user.clear(screen.getByRole("combobox"));
    await user.keyboard("{Enter}");
    expect(screen.getByRole("status", { name: "Selected scopes" })).toHaveTextContent(/^changed added$/);
  });

  it("respects external reset while a staged draft is pending", async () => {
    function Editor() {
      const [values, setValues] = useState(["roles"]);
      return <><AsyncOptionSelect optionSet="service-scope" context={{ owner_id: "owner", kind: "service-scope", principal_type: "service_account" }} label="Scopes" value={values} onChange={setValues} allowCustom /><button onClick={() => setValues(["proxy"])}>Reset scope form</button><button onClick={() => setValues(["roles", "unsaved"])}>Restore former values</button><output aria-label="Stored">{values.join(" ")}</output></>;
    }
    const user = userEvent.setup();
    render(<Editor />, { wrapper: optionsWrapper() });
    await user.type(screen.getByRole("combobox"), "unsaved");
    await user.click(screen.getByRole("button", { name: "Reset scope form" }));
    expect(screen.getByRole("combobox")).toHaveValue("");
    expect(screen.queryByRole("button", { name: "Edit roles" })).not.toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Restore former values" }));
    expect(screen.getByRole("combobox")).toHaveValue("");
    expect(screen.getByRole("button", { name: "Edit unsaved" })).toBeInTheDocument();
    await user.type(screen.getByRole("combobox"), "new{Enter}");
    expect(screen.getByRole("status", { name: "Stored" })).toHaveTextContent(/^roles unsaved new$/);
  });

  it.each([400, 403, 404])("clears loaded suggestions on a next-page context rejection (%i)", async (status) => {
    fetchMock.mockImplementation(async (url) => {
      const offset = new URL(String(url), "http://localhost").searchParams.get("offset");
      if (offset !== "0") return new Response(JSON.stringify({ message: "Access changed" }), { status });
      return new Response(JSON.stringify(optionsResponse(String(url), { total: 200, next_offset: 100 })));
    });
    render(<Picker custom initial={["custom:retained"]} />, { wrapper: optionsWrapper() });
    await userEvent.click(screen.getByRole("combobox"));
    await screen.findByRole("alert");
    expect(screen.queryByRole("option")).not.toBeInTheDocument();
    expect(fetchMock).toHaveBeenCalledTimes(2);
    await userEvent.type(screen.getByRole("combobox"), "custom:new{Enter}");
    expect(screen.getByRole("button", { name: "Edit custom:new" })).toBeInTheDocument();
  });

  it("clears suggestions when revalidation fails and stops invalid pagination", async () => {
    function Editor() {
      const client = useQueryClient();
      return <><Picker custom /><button onClick={() => void client.invalidateQueries({ queryKey: ["options"] })}>Refresh suggestions</button></>;
    }
    const user = userEvent.setup();
    render(<Editor />, { wrapper: optionsWrapper() });
    await user.click(screen.getByRole("combobox"));
    await screen.findByRole("option", { name: "proxy" });
    fetchMock.mockImplementation(async () => new Response(JSON.stringify({ message: "Access changed" }), { status: 403 }));
    await user.click(screen.getByRole("button", { name: "Refresh suggestions" }));
    await user.click(screen.getByRole("combobox"));
    await screen.findByRole("alert");
    expect(screen.queryByRole("option")).not.toBeInTheDocument();
    const before = fetchMock.mock.calls.length;
    fetchMock.mockImplementation(async (url) => new Response(JSON.stringify(optionsResponse(String(url), { next_offset: 0 }))));
    await user.click(screen.getByRole("button", { name: "Retry suggestions" }));
    await waitFor(() => expect(fetchMock.mock.calls.length).toBe(before + 1));
    await screen.findByRole("alert");
    expect(screen.queryByRole("option")).not.toBeInTheDocument();
  });

  it("reopens suggestions when the already focused add input is clicked after removing a pill", async () => {
    const user = userEvent.setup();
    render(<Picker custom initial={["roles"]} />, { wrapper: optionsWrapper() });
    const input = screen.getByRole("combobox");
    await user.click(input);
    await screen.findByRole("option", { name: "proxy" });
    await user.keyboard("{Escape}");
    await user.click(screen.getByRole("button", { name: "Remove roles" }));
    expect(input).toHaveFocus();
    await user.click(input);
    await user.click(await screen.findByRole("option", { name: "roles" }));
    expect(screen.getByRole("button", { name: "Edit roles" })).toBeInTheDocument();
  });

});

function HistoryPicker() {
  const [values, setValues] = useState<string[]>([]);
  return <><AsyncOptionSelect optionSet="service-history-action" context={{ kind: "service-history" }} label="History actions" value={values} onChange={setValues} /><output aria-label="History filter">{values.join(",")}</output></>;
}
function staticHistoryOptions() {
  return { option_set: "service-history-action", items: [{ value: "service.created", label: "Service created", description: "Created", group: "History", source: "backend_definition", owner_id: null, resource_id: null, disabled: false, disabled_reason: null }], total: 1, next_offset: null, version: "history-v1", freshness: { definitions_version: "v1", resources: "static", evaluated_at: "2026-09-17T00:00:00Z", max_age_seconds: 0 } };
}
describe("static history options", () => {
  it("uses no fabricated owner context, keeps labels, and rejects custom input", async () => {
    fetchMock.mockImplementation(async () => new Response(JSON.stringify(staticHistoryOptions())));
    render(<HistoryPicker />, { wrapper: optionsWrapper() });
    const input = screen.getByRole("combobox");
    expect(input).toHaveAttribute("placeholder", "Search choices…");
    await userEvent.click(input);
    await userEvent.click(await screen.findByRole("option", { name: "Service created" }));
    expect(screen.getByRole("status", { name: "History filter" })).toHaveTextContent("service.created");
    expect(screen.getByText("Service created")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Edit service.created" })).not.toBeInTheDocument();
    await userEvent.type(input, "made-up.action{Enter}");
    expect(screen.getByRole("status", { name: "History filter" })).toHaveTextContent(/^service.created$/);
    for (const [url] of fetchMock.mock.calls) {
      const params = new URL(String(url), "http://localhost").searchParams;
      expect(params.has("owner_id")).toBe(false);
      expect(params.has("principal_type")).toBe(false);
      expect(params.has("service_account_id")).toBe(false);
    }
  });

  it.each([1, 2])("rejects next_offset=%s at or beyond total and hides invalid choices", async (offset) => {
    fetchMock.mockImplementation(async () => new Response(JSON.stringify({ ...staticHistoryOptions(), next_offset: offset })));
    render(<HistoryPicker />, { wrapper: optionsWrapper() });
    await userEvent.click(screen.getByRole("combobox"));
    expect(await screen.findByRole("alert")).toHaveTextContent("Suggestions unavailable");
    expect(screen.queryByRole("option")).not.toBeInTheDocument();
  });

  it("rejects a live resource response in a static set", async () => {
    const response = staticHistoryOptions();
    fetchMock.mockImplementation(async () => new Response(JSON.stringify({ ...response, freshness: { ...response.freshness, resources: "live" } })));
    render(<HistoryPicker />, { wrapper: optionsWrapper() });
    await userEvent.click(screen.getByRole("combobox"));
    expect(await screen.findByRole("alert")).toHaveTextContent("Suggestions unavailable");
  });
});
