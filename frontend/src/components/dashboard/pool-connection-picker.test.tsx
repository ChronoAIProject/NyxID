import { useState } from "react";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogTitle,
} from "@/components/ui/dialog";
import type { PoolCandidate } from "@/schemas/pools";
import { PoolConnectionPicker } from "./pool-connection-picker";

const candidate: PoolCandidate = {
  user_service_id: "one",
  name: "Primary connection",
  slug: "primary",
  is_active: true,
  credential_binding: "user",
  protocol: null,
  eligible: true,
  reason: null,
  catalog_service_id: "catalog",
  requires_compatibility_declaration: false,
  cooldown_until: null,
  consecutive_failures: 0,
  last_status: null,
};
const backup = {
  ...candidate,
  user_service_id: "two",
  name: "Backup connection",
};
const failed = {
  ...candidate,
  user_service_id: "failed",
  name: "Failed connection",
  eligible: false,
  reason: "credential_unavailable",
};
const defaults = {
  rows: [candidate, backup, failed],
  selectedIds: [],
  search: "",
  onSearch: vi.fn(),
  onToggle: vi.fn(),
  isLoading: false,
  isSearching: false,
  isError: false,
  error: null,
  onRetry: vi.fn(),
  hasNextPage: false,
  isFetchingNextPage: false,
  onLoadMore: vi.fn(),
};
function Harness({
  initialIds = [],
  sourceRows = defaults.rows,
}: {
  initialIds?: string[];
  sourceRows?: PoolCandidate[];
}) {
  const [ids, setIds] = useState(initialIds);
  const [search, setSearch] = useState("");
  return (
    <Dialog defaultOpen>
      <DialogContent>
        <DialogTitle>Pool editor</DialogTitle>
        <DialogDescription>Configure your pool</DialogDescription>
        <PoolConnectionPicker
          {...defaults}
          selectedIds={ids}
          search={search}
          onSearch={setSearch}
          rows={sourceRows.filter((row) =>
            `${row.name || row.slug} ${row.slug} ${row.group_name ?? ""}`
              .toLowerCase()
              .includes(search.toLowerCase()),
          )}
          onToggle={(row) =>
            setIds((current) =>
              current.includes(row.user_service_id)
                ? current.filter((id) => id !== row.user_service_id)
                : [...current, row.user_service_id],
            )
          }
        />
      </DialogContent>
    </Dialog>
  );
}
async function openPicker(user: ReturnType<typeof userEvent.setup>) {
  const trigger = screen.getByRole("button", { name: "Choose connections" });
  await user.click(trigger);
  const input = screen.getByRole("combobox", {
    name: "Search candidate services",
  });
  await waitFor(() => expect(input).toHaveFocus());
  return { trigger, input };
}
describe("pool connection picker", () => {
  it("selects and deselects multiple rows without closing; search retains selection", async () => {
    const user = userEvent.setup();
    render(<Harness />);
    const { input, trigger } = await openPicker(user);
    expect(
      screen.getByRole("listbox", { name: "Connections" }),
    ).toHaveAttribute("aria-multiselectable", "true");
    await user.click(screen.getByRole("option", { name: candidate.name }));
    await user.click(screen.getByRole("option", { name: backup.name }));
    expect(
      screen.getByRole("option", { name: candidate.name }),
    ).toHaveAttribute("aria-selected", "true");
    expect(screen.getByRole("status")).toHaveTextContent("2 of 50 selected");
    expect(input).toHaveFocus();
    await user.type(input, "Backup");
    expect(
      screen.queryByRole("option", { name: candidate.name }),
    ).not.toBeInTheDocument();
    expect(screen.getByRole("option", { name: backup.name })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    await user.clear(input);
    expect(
      screen.getByRole("option", { name: candidate.name }),
    ).toHaveAttribute("aria-selected", "true");
    await user.click(screen.getByRole("option", { name: backup.name }));
    expect(screen.getByRole("option", { name: backup.name })).toHaveAttribute(
      "aria-selected",
      "false",
    );
    await user.click(screen.getByRole("button", { name: "Done" }));
    await waitFor(() => expect(trigger).toHaveFocus());
    await user.click(trigger);
    expect(
      screen.getByRole("option", { name: candidate.name }),
    ).toHaveAttribute("aria-selected", "true");
    for (let cycle = 0; cycle < 3; cycle++) {
      await user.click(screen.getByRole("option", { name: backup.name }));
      await user.click(screen.getByRole("option", { name: backup.name }));
      await user.keyboard("{Escape}");
      await waitFor(() => expect(trigger).toHaveFocus());
      expect(trigger).toHaveTextContent("1 connection selected");
      await user.click(trigger);
      expect(screen.getByRole("option", { name: backup.name })).toHaveAttribute(
        "aria-selected",
        "false",
      );
    }
  });
  it("supports arrow/enter toggling, announces disabled reasons, and Escape closes only the picker then restores focus", async () => {
    const user = userEvent.setup();
    render(<Harness />);
    const { trigger, input } = await openPicker(user);
    await user.keyboard(
      "{ArrowDown}{Enter}{ArrowDown}{Enter}{ArrowDown}{Enter}",
    );
    expect(screen.getByRole("status")).toHaveTextContent("2 of 50 selected");
    const failedOption = screen.getByRole("option", { name: failed.name });
    expect(input).toHaveAttribute("aria-activedescendant", failedOption.id);
    expect(failedOption).toHaveAttribute("aria-disabled", "true");
    expect(failedOption).toHaveAccessibleDescription(
      /Credentials unavailable.*Reconnect/,
    );
    expect(failedOption).toHaveAttribute("aria-selected", "false");
    await user.keyboard("{Escape}");
    await waitFor(() => expect(trigger).toHaveFocus());
    expect(screen.queryByRole("combobox")).not.toBeInTheDocument();
    expect(screen.getByRole("dialog", { name: "Pool editor" })).toBeVisible();
  });
  it("enforces 50 connections while allowing deselection, including an unavailable selected row", async () => {
    const user = userEvent.setup();
    render(
      <Harness
        initialIds={[
          "one",
          "failed",
          ...Array.from({ length: 48 }, (_, n) => `other-${n}`),
        ]}
      />,
    );
    await openPicker(user);
    expect(screen.getByRole("option", { name: backup.name })).toHaveAttribute(
      "aria-disabled",
      "true",
    );
    await user.click(screen.getByRole("option", { name: backup.name }));
    expect(screen.getByRole("status")).toHaveTextContent("50 of 50 selected");
    await user.click(screen.getByRole("option", { name: failed.name }));
    expect(screen.getByRole("status")).toHaveTextContent("49 of 50 selected");
    expect(screen.getByRole("option", { name: backup.name })).toHaveAttribute(
      "aria-disabled",
      "false",
    );
    expect(screen.getByRole("option", { name: failed.name })).toHaveAttribute(
      "aria-disabled",
      "true",
    );
  });
  it("groups interleaved catalog rows, de-duplicates page overlap, and navigates display order", async () => {
    const user = userEvent.setup();
    const groupedRows: PoolCandidate[] = [
      {
        ...candidate,
        name: "A account 1",
        catalog_service_id: "catalog-a",
        group_name: "Shared service",
        group_slug: "service-a",
      },
      {
        ...backup,
        name: "B account 1",
        catalog_service_id: "catalog-b",
        group_name: "Shared service",
        group_slug: "service-b",
      },
      {
        ...candidate,
        user_service_id: "three",
        name: "A account 2",
        catalog_service_id: "catalog-a",
        group_name: "Shared service",
        group_slug: "service-a",
      },
      {
        ...candidate,
        user_service_id: "three",
        name: "A account 2 duplicate",
        catalog_service_id: "catalog-a",
        group_name: "Shared service",
        group_slug: "service-a",
      },
    ];
    render(<Harness sourceRows={groupedRows} />);
    const { input } = await openPicker(user);
    expect(
      screen.getByRole("group", {
        name: "Shared service (service-a) (2 loaded)",
      }),
    ).toBeVisible();
    expect(
      screen.getByRole("group", {
        name: "Shared service (service-b) (1 loaded)",
      }),
    ).toBeVisible();
    expect(screen.getAllByRole("option")).toHaveLength(3);
    await user.keyboard("{ArrowUp}");
    expect(input).toHaveAttribute(
      "aria-activedescendant",
      screen.getByRole("option", { name: "B account 1" }).id,
    );
    await user.keyboard("{Enter}");
    expect(screen.getByRole("option", { name: "B account 1" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    await user.keyboard("{ArrowDown}{Enter}{ArrowDown}{Enter}");
    expect(screen.getByRole("option", { name: "A account 1" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    expect(screen.getByRole("option", { name: "A account 2" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    expect(input).toHaveFocus();
  });
  it("keeps retry and paging inside the dropdown and distinguishes loading, empty search, and empty inventory", async () => {
    const user = userEvent.setup();
    const retry = vi.fn();
    const more = vi.fn();
    const view = render(
      <PoolConnectionPicker {...defaults} rows={[]} isLoading />,
    );
    await openPicker(user);
    expect(screen.getByText("Loading connections…")).toBeVisible();
    expect(screen.queryByText(/No connections/)).not.toBeInTheDocument();
    view.rerender(
      <PoolConnectionPicker
        {...defaults}
        rows={[]}
        isError
        error={new Error("Temporary inventory failure")}
        onRetry={retry}
      />,
    );
    expect(screen.getByRole("alert")).toHaveTextContent(
      "Temporary inventory failure",
    );
    await user.click(screen.getByRole("button", { name: "Retry" }));
    expect(retry).toHaveBeenCalledTimes(1);
    view.rerender(
      <PoolConnectionPicker {...defaults} rows={[]} search="missing" />,
    );
    expect(screen.getByText("No connections match this search.")).toBeVisible();
    view.rerender(<PoolConnectionPicker {...defaults} rows={[]} />);
    expect(screen.getByText(/No connections yet/)).toBeVisible();
    view.rerender(
      <PoolConnectionPicker {...defaults} hasNextPage onLoadMore={more} />,
    );
    await user.click(
      screen.getByRole("button", { name: "Load more connections" }),
    );
    expect(more).toHaveBeenCalledTimes(1);
    expect(screen.getByRole("combobox")).toBeVisible();
  });
});

it("returns keyboard focus to search after the final page button disappears", async () => {
  const user = userEvent.setup();
  const { rerender } = render(
    <PoolConnectionPicker {...defaults} hasNextPage />,
  );
  const { input } = await openPicker(user);
  await user.tab();
  const more = screen.getByRole("button", { name: "Load more connections" });
  expect(more).toHaveFocus();
  await user.keyboard("{Enter}");
  rerender(
    <PoolConnectionPicker {...defaults} hasNextPage isFetchingNextPage />,
  );
  rerender(
    <PoolConnectionPicker
      {...defaults}
      rows={[...defaults.rows, { ...backup, user_service_id: "last" }]}
      hasNextPage={false}
    />,
  );
  await waitFor(() => expect(input).toHaveFocus());
});

it("blocks stale additions and pagination during compatibility checks while allowing removal", async () => {
  const user = userEvent.setup();
  const onToggle = vi.fn();
  const onLoadMore = vi.fn();
  const { rerender } = render(
    <PoolConnectionPicker
      {...defaults}
      selectedIds={[candidate.user_service_id]}
      onToggle={onToggle}
      onLoadMore={onLoadMore}
      hasNextPage
      isCheckingCompatibility
      isRefreshing
    />,
  );
  await openPicker(user);
  expect(screen.getByText("Checking compatibility…")).toBeVisible();
  const primary = screen.getByRole("option", { name: candidate.name! });
  const second = screen.getByRole("option", { name: backup.name });
  expect(primary).toHaveAttribute("aria-disabled", "false");
  expect(second).toHaveAttribute("aria-disabled", "true");
  await user.click(second);
  await user.click(
    screen.getByRole("button", { name: "Load more connections" }),
  );
  expect(onToggle).not.toHaveBeenCalled();
  expect(onLoadMore).not.toHaveBeenCalled();
  await user.click(primary);
  expect(onToggle).toHaveBeenCalledWith(candidate);
  rerender(
    <PoolConnectionPicker {...defaults} onLoadMore={onLoadMore} hasNextPage />,
  );
  await user.click(
    screen.getByRole("button", { name: "Load more connections" }),
  );
  expect(onLoadMore).toHaveBeenCalledTimes(1);
});
