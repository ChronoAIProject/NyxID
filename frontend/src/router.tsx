import {
  createRouter,
  createRoute,
  createRootRoute,
  redirect,
  Outlet,
} from "@tanstack/react-router"
import { TooltipProvider } from "@/components/ui/tooltip"
import { Toaster } from "@/components/ui/toast"
import { AuthLayout } from "@/components/layout/auth-layout"
import { DashboardLayout } from "@/components/layout/dashboard-layout"
import { useAuthStore } from "@/stores/auth-store"

import { LoginPage } from "@/pages/login"
import { RegisterPage } from "@/pages/register"
import { DashboardPage } from "@/pages/dashboard"
import { ApiKeysPage } from "@/pages/api-keys"
import { ServicesPage } from "@/pages/services"
import { ConnectionsPage } from "@/pages/connections"
import { SettingsPage } from "@/pages/settings"

const rootRoute = createRootRoute({
  component: () => (
    <TooltipProvider>
      <Outlet />
      <Toaster />
    </TooltipProvider>
  ),
})

const authLayout = createRoute({
  id: "auth",
  getParentRoute: () => rootRoute,
  beforeLoad: () => {
    const { isAuthenticated, isLoading } = useAuthStore.getState()
    if (isAuthenticated && !isLoading) {
      throw redirect({ to: "/" })
    }
  },
  component: AuthLayout,
})

const loginRoute = createRoute({
  path: "/login",
  getParentRoute: () => authLayout,
  component: LoginPage,
})

const registerRoute = createRoute({
  path: "/register",
  getParentRoute: () => authLayout,
  component: RegisterPage,
})

const dashboardLayout = createRoute({
  id: "dashboard",
  getParentRoute: () => rootRoute,
  beforeLoad: () => {
    const { isAuthenticated, isLoading } = useAuthStore.getState()
    if (!isAuthenticated && !isLoading) {
      throw redirect({ to: "/login" })
    }
  },
  component: DashboardLayout,
})

const dashboardIndexRoute = createRoute({
  path: "/",
  getParentRoute: () => dashboardLayout,
  component: DashboardPage,
})

const apiKeysRoute = createRoute({
  path: "/api-keys",
  getParentRoute: () => dashboardLayout,
  component: ApiKeysPage,
})

const servicesRoute = createRoute({
  path: "/services",
  getParentRoute: () => dashboardLayout,
  component: ServicesPage,
})

const connectionsRoute = createRoute({
  path: "/connections",
  getParentRoute: () => dashboardLayout,
  component: ConnectionsPage,
})

const settingsRoute = createRoute({
  path: "/settings",
  getParentRoute: () => dashboardLayout,
  component: SettingsPage,
})

const routeTree = rootRoute.addChildren([
  authLayout.addChildren([loginRoute, registerRoute]),
  dashboardLayout.addChildren([
    dashboardIndexRoute,
    apiKeysRoute,
    servicesRoute,
    connectionsRoute,
    settingsRoute,
  ]),
])

export const router = createRouter({
  routeTree,
  defaultPreload: "intent",
})

declare module "@tanstack/react-router" {
  interface Register {
    router: typeof router
  }
}
