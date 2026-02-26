import { Suspense, useState, useCallback } from "react";
import { Outlet } from "@tanstack/react-router";
import { Sidebar } from "@/components/dashboard/sidebar";
import { Header } from "@/components/dashboard/header";

export function DashboardLayout() {
  const [sidebarOpen, setSidebarOpen] = useState(false);

  const openSidebar = useCallback(() => setSidebarOpen(true), []);
  const closeSidebar = useCallback(() => setSidebarOpen(false), []);

  return (
    <div
      className="flex h-dvh overflow-hidden bg-background"
      style={{
        paddingTop: "var(--sat)",
        paddingLeft: "var(--sal)",
        paddingRight: "var(--sar)",
      }}
    >
      {/* ── Desktop sidebar (always visible at md+) ── */}
      <div className="hidden md:flex">
        <Sidebar />
      </div>

      {/* ── Mobile drawer overlay ── */}
      {sidebarOpen && (
        <div className="fixed inset-0 z-40 flex md:hidden" style={{ paddingTop: "var(--sat)" }}>
          <div
            className="fixed inset-0 bg-black/60 backdrop-blur-sm"
            onClick={closeSidebar}
            onKeyDown={(e) => { if (e.key === "Escape") closeSidebar(); }}
            role="button"
            tabIndex={-1}
            aria-label="Close navigation"
          />
          <div className="relative z-50 w-[280px] animate-in slide-in-from-left duration-200">
            <Sidebar onNavigate={closeSidebar} />
          </div>
        </div>
      )}

      <div className="flex flex-1 flex-col overflow-hidden">
        <Header onMenuClick={openSidebar} />
        <main
          className="flex-1 overflow-x-hidden overflow-y-auto overscroll-contain px-4 py-6 md:px-14 md:py-12"
          style={{ paddingBottom: "max(1.5rem, var(--sab))" }}
        >
          <Suspense>
            <Outlet />
          </Suspense>
        </main>
      </div>
    </div>
  );
}
