import { formatDateTime } from "@/lib/utils";
import { it, expect, vi } from "vitest";
import { fireEvent, render, screen, within } from "@testing-library/react";
import {
  createMemoryHistory,
  createRootRoute,
  createRoute,
  createRouter,
  RouterProvider,
} from "@tanstack/react-router";
import { AdminCrashReportsPage, CrashReportView } from "./admin-crash-reports";
import { crashReportSchema } from "@/schemas/admin-crash-reports";
import {
  useCrashSummary,
  useCrashReports,
} from "@/hooks/use-admin-crash-reports";
vi.mock("@/hooks/use-admin-crash-reports", () => ({
  useCrashSummary: vi.fn(),
  useCrashReports: vi.fn(),
  useCrashReport: vi.fn(),
}));
it("admin report details escape every server string as text", () => {
  const attack = "<img src=x onerror=alert(1)>";
  const report = crashReportSchema.parse({
    id: attack,
    event_id: "event",
    captured_at: "2026-11-14T00:00:00Z",
    platform: "web",
    kind: "js_error",
    exception_class: attack,
    native_signal: null,
    app_version: "1",
    app_build: "2",
    artifact_id: null,
    os_version: "1",
    runtime: null,
    js_frames: [{ asset: attack, line: 1, column: 2 }],
    native_images: [{ uuid: "image-uuid", name: attack, size: 4096 }],
    native_frames: [{ image: 0, offset: 32 }],
    environment: attack,
    severity: "error",
    fingerprint: attack,
    received_at: "today",
    expires_at: "tomorrow",
    export: {
      status: "disabled",
      destination_id: null,
      attempts: 0,
      last_error_code: null,
      exported_at: null,
      provider_event_id: "event",
    },
  });
  const { container } = render(<CrashReportView report={report} />);
  expect(screen.getAllByText(attack).length).toBeGreaterThan(0);
  expect(container.querySelector("img")).toBeNull();
  expect(
    screen.getByText(formatDateTime("2026-11-14T00:00:00Z")),
  ).toBeInTheDocument();
  expect(screen.getByText(/0x1000/)).toBeInTheDocument();
  expect(screen.getByText(/0x20/)).toBeInTheDocument();
  expect(container.innerHTML).not.toContain("user_id");
});

it("admin overview renders ingestion health and localized filter labels", () => {
  vi.mocked(useCrashSummary).mockReturnValue({
    isLoading: false,
    error: null,
    data: {
      health: {
        outcomes: { accepted: 3 },
        export_statuses: { disabled: 3 },
        destination_ids: [],
        retention_days: 30,
        flag_enabled: false,
      },
      versions: [],
    },
  } as unknown as ReturnType<typeof useCrashSummary>);
  vi.mocked(useCrashReports).mockReturnValue({
    isLoading: false,
    error: null,
    data: { reports: [], cursor: null },
  } as unknown as ReturnType<typeof useCrashReports>);
  render(<AdminCrashReportsPage />);
  expect(
    screen.getByRole("columnheader", { name: "Reported events by kind" }),
  ).toBeInTheDocument();
  expect(screen.getByText("Ingestion health")).toBeInTheDocument();
  expect(screen.getByRole("option", { name: "Last 24 hours" })).toHaveValue(
    "24h",
  );
  expect(screen.getByRole("option", { name: "Last 7 days" })).toHaveValue("7d");
  expect(screen.getByRole("option", { name: "Last 30 days" })).toHaveValue(
    "30d",
  );
  expect(screen.getByRole("option", { name: "iOS" })).toHaveValue("ios");
  expect(screen.getByText("No reports in this selection.")).toBeInTheDocument();
});

it("mobile cards and desktop tables expose the same reports and version filters", async () => {
  const version = {
    platform: "ios",
    environment: "test",
    app_version: "1.2.3",
    app_build: "42",
    artifact_id: "artifact",
    counts: { native_crash: 2 },
    total: 2,
    first_seen: "2026-11-14T00:00:00Z",
    last_seen: "2026-11-14T01:00:00Z",
  };
  vi.mocked(useCrashSummary).mockReturnValue({
    isLoading: false,
    error: null,
    data: {
      health: {
        outcomes: { accepted: 2 },
        export_statuses: { disabled: 2 },
        destination_ids: [],
        retention_days: 30,
        flag_enabled: true,
      },
      versions: [version],
    },
  } as unknown as ReturnType<typeof useCrashSummary>);
  vi.mocked(useCrashReports).mockReturnValue({
    isLoading: false,
    error: null,
    data: {
      reports: [
        {
          id: "report-42",
          captured_at: version.last_seen,
          platform: "ios",
          app_version: version.app_version,
          app_build: version.app_build,
          kind: "native_crash",
          export: { status: "disabled" },
        },
      ],
      cursor: null,
    },
  } as unknown as ReturnType<typeof useCrashReports>);
  const root = createRootRoute({ component: AdminCrashReportsPage });
  const detail = createRoute({
    getParentRoute: () => root,
    path: "/admin/crash-reports/$id",
    component: () => null,
  });
  const router = createRouter({
    routeTree: root.addChildren([detail]),
    history: createMemoryHistory({ initialEntries: ["/"] }),
  });
  await router.load();
  render(<RouterProvider router={router} />);
  const cards = within(
    await screen.findByRole("list", { name: "Reported events by version" }),
  );
  const table = within(
    screen.getByRole("table", { name: "Reported events by version" }),
  );
  for (const view of [cards, table]) {
    expect(view.getByText("artifact")).toBeInTheDocument();
    expect(view.getByText("iOS native crash: 2")).toBeInTheDocument();
    expect(
      view.getByText(formatDateTime(version.last_seen)),
    ).toBeInTheDocument();
  }
  fireEvent.click(cards.getByRole("button", { name: "iOS · 1.2.3 (42)" }));
  expect(vi.mocked(useCrashReports)).toHaveBeenLastCalledWith({
    platform: "ios",
    app_version: "1.2.3",
    app_build: "42",
  });
  fireEvent.click(table.getByRole("button", { name: "1.2.3" }));
  expect(vi.mocked(useCrashReports)).toHaveBeenLastCalledWith({
    platform: "ios",
    app_version: "1.2.3",
    app_build: "42",
  });
  for (const role of ["list", "table"] as const) {
    const reports = within(
      screen.getByRole(role, { name: "Individual reports" }),
    );
    expect(
      reports.getByRole("link", { name: formatDateTime(version.last_seen) }),
    ).toHaveAttribute("href", "/admin/crash-reports/report-42");
    expect(reports.getByText("Disabled")).toBeInTheDocument();
  }
});
