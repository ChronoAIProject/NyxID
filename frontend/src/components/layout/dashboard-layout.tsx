import { Suspense } from "react";
import { Outlet } from "@tanstack/react-router";
import { Sidebar } from "@/components/dashboard/sidebar";
import { Header } from "@/components/dashboard/header";

export function DashboardLayout() {
  return (
    <div className="flex h-screen overflow-hidden bg-background">
      <Sidebar />
      <div className="flex flex-1 flex-col overflow-hidden">
        <Header />
        <main className="flex-1 overflow-y-auto overscroll-contain px-14 py-12">
          <Suspense>
            <Outlet />
          </Suspense>
        </main>
      </div>
    </div>
  );
}
