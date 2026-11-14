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
import {
  crashReportSchema,
  crashSummarySchema,
} from "@/schemas/admin-crash-reports";
import {
  useCrashSummary,
  useCrashReports,
} from "@/hooks/use-admin-crash-reports";
vi.mock("@/hooks/use-admin-crash-reports", () => ({
  useCrashSummary: vi.fn(),
  useCrashReports: vi.fn(),
  useCrashReport: vi.fn(),
  useRegisterCrashRelease: vi.fn(() => ({
    isPending: false,
    isError: false,
    mutate: vi.fn(),
  })),
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
    release_verified: false,
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
  expect(screen.getByText("Unverified release")).toBeInTheDocument();
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
        rollout: "off",
        export_enabled: false,
      },
      versions: [],
      unverified_releases: [],
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
  const exportStatus = within(
    screen.getByRole("region", { name: "Export status" }),
  );
  expect(
    exportStatus.getByRole("heading", { name: "Export status" }),
  ).toBeInTheDocument();
  expect(exportStatus.getByText("Disabled")).toBeInTheDocument();
  expect(exportStatus.getByText("3")).toBeInTheDocument();
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

it.each([
  { artifactIds: [] },
  { artifactIds: ["artifact-a"] },
  { artifactIds: ["artifact-a", "artifact-b"] },
])(
  "mobile cards and desktop tables show artifacts $artifactIds and preserve version filters",
  async ({ artifactIds }) => {
    const version = {
      platform: "ios",
      environment: "test",
      app_version: "1.0.4",
      app_build: "45",
      artifact_id: artifactIds.length === 1 ? artifactIds[0] : null,
      artifact_ids: artifactIds,
      counts: { native_crash: 1, js_fatal: 1, unhandled_rejection: 1 },
      total: 3,
      first_seen: "2026-11-14T00:00:00Z",
      last_seen: "2026-11-14T01:00:00Z",
    };
    vi.mocked(useCrashSummary).mockReturnValue({
      isLoading: false,
      error: null,
      data: crashSummarySchema.parse({
        health: {
          outcomes: { accepted: 3 },
          export_statuses: { disabled: 3 },
          destination_ids: [],
          retention_days: 30,
          flag_enabled: true,
          rollout: "cohort",
          export_enabled: false,
        },
        versions: [version],
        unverified_releases: [],
      }),
    } as unknown as ReturnType<typeof useCrashSummary>);
    vi.mocked(useCrashReports).mockReturnValue({
      isLoading: false,
      error: null,
      data: {
        reports: [
          {
            id: "report-45",
            captured_at: version.last_seen,
            platform: "ios",
            app_version: version.app_version,
            app_build: version.app_build,
            kind: "native_crash",
            release_verified: false,
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
    expect(cards.getAllByRole("listitem")).toHaveLength(1);
    expect(table.getAllByRole("row")).toHaveLength(2);
    for (const view of [cards, table]) {
      expect(view.getByText(artifactIds.join(", ") || "—")).toBeInTheDocument();
      expect(
        view.getByText(
          "iOS native crash: 1, JavaScript fatal error: 1, Unhandled rejection: 1",
        ),
      ).toBeInTheDocument();
      expect(
        view.getByText(formatDateTime(version.last_seen)),
      ).toBeInTheDocument();
    }
    fireEvent.click(cards.getByRole("button", { name: "iOS · 1.0.4 (45)" }));
    expect(vi.mocked(useCrashReports)).toHaveBeenLastCalledWith({
      platform: "ios",
      app_version: "1.0.4",
      app_build: "45",
    });
    fireEvent.click(table.getByRole("button", { name: "1.0.4" }));
    expect(vi.mocked(useCrashReports)).toHaveBeenLastCalledWith({
      platform: "ios",
      app_version: "1.0.4",
      app_build: "45",
    });
    for (const role of ["list", "table"] as const) {
      const reports = within(
        screen.getByRole(role, { name: "Individual reports" }),
      );
      const timestamp = reports.getByRole("link", {
        name: formatDateTime(version.last_seen),
      });
      expect(timestamp).toHaveAttribute("href", "/admin/crash-reports/report-45");
      expect(timestamp).toHaveClass("font-mono");
      expect(reports.getByText("Disabled")).toBeInTheDocument();
      const badge = reports.getByText("Unverified release");
      expect(badge).toBeInTheDocument();
      expect(badge.closest(".font-mono")).toBeNull();
    }
  },
);

it.each(["admin", "operator"])(
  "%s sees unverified counts, and only an admin can register",
  async (role) => {
    const { useAuthStore } = await import("@/stores/auth-store");
    const { useRegisterCrashRelease } =
      await import("@/hooks/use-admin-crash-reports");
    const previous = useAuthStore.getState().user;
    useAuthStore.setState({
      user: { id: "actor", role } as NonNullable<typeof previous>,
    });
    const mutate = vi.fn();
    vi.mocked(useRegisterCrashRelease).mockReturnValue({
      mutate,
      isPending: false,
      isError: false,
    } as unknown as ReturnType<typeof useRegisterCrashRelease>);
    vi.mocked(useCrashSummary).mockReturnValue({
      isLoading: false,
      error: null,
      data: crashSummarySchema.parse({
        health: {
          outcomes: {},
          export_statuses: {},
          destination_ids: [],
          retention_days: 30,
          flag_enabled: false,
          rollout: "killed",
          export_enabled: false,
        },
        versions: [],
        unverified_releases: [
          {
            platform: "ios",
            environment: "test",
            app_version: "1.0.4",
            app_build: "45",
            artifact_id: null,
            artifact_ids: [],
            counts: { native_crash: 3 },
            total: 3,
            first_seen: "2026-11-14T00:00:00Z",
            last_seen: "2026-11-14T00:00:00Z",
          },
        ],
      }),
    } as unknown as ReturnType<typeof useCrashSummary>);
    vi.mocked(useCrashReports).mockReturnValue({
      isLoading: false,
      error: null,
      data: { reports: [], cursor: null },
    } as unknown as ReturnType<typeof useCrashReports>);
    render(<AdminCrashReportsPage />);
    const list = within(
      screen.getByRole("list", { name: "Unverified releases" }),
    );
    expect(list.getAllByRole("listitem")).toHaveLength(1);
    expect(screen.getByText(/Rollout killed/)).toBeInTheDocument();
    expect(screen.getByText("Export disabled")).toBeInTheDocument();
    if (role === "admin") {
      fireEvent.click(list.getByRole("button", { name: "Register release" }));
      expect(mutate).toHaveBeenCalledWith({
        platform: "ios",
        app_version: "1.0.4",
        app_build: "45",
      });
    } else
      expect(
        list.queryByRole("button", { name: "Register release" }),
      ).not.toBeInTheDocument();
    useAuthStore.setState({ user: previous });
  },
);
