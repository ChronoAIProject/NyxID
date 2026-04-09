/// <reference types="vitest/config" />
import { defineConfig } from "vite"
import react from "@vitejs/plugin-react"
import tailwindcss from "@tailwindcss/vite"
import path from "path"

const backendUrl = process.env.BACKEND_URL || "http://localhost:3001"

/** Strip Secure / Domain from Set-Cookie so cookies work on http://localhost */
function cookieRewrite(proxyRes: import("http").IncomingMessage) {
  const sc = proxyRes.headers["set-cookie"]
  if (!sc) return
  proxyRes.headers["set-cookie"] = sc.map((c) =>
    c
      .replace(/;\s*Secure/gi, "")
      .replace(/;\s*Domain=[^;]*/gi, "")
      .replace(/;\s*SameSite=None/gi, "; SameSite=Lax"),
  )
}

const proxyTarget = {
  target: backendUrl,
  changeOrigin: true,
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  configure: (proxy: any) => {
    proxy.on("proxyRes", cookieRewrite)
  },
}

const apiProxy = {
  "/api": proxyTarget,
  "^/oauth(?:/.*)?$": proxyTarget,
  "/mcp": proxyTarget,
  "/.well-known": proxyTarget,
  "/health": proxyTarget,
}

export default defineConfig({
  plugins: [react(), tailwindcss()],
  resolve: {
    alias: {
      "@": path.resolve(__dirname, "./src"),
    },
  },
  build: {
    rollupOptions: {
      output: {
        manualChunks(id) {
          if (id.includes("node_modules/react-dom") || id.includes("node_modules/react/")) {
            return "vendor-react"
          }
          if (id.includes("node_modules/@tanstack/react-router")) {
            return "vendor-router"
          }
          if (id.includes("node_modules/@tanstack/react-query")) {
            return "vendor-query"
          }
          if (id.includes("node_modules/@radix-ui/")) {
            return "vendor-radix"
          }
        },
      },
    },
  },
  server: {
    port: 5173,
    proxy: apiProxy,
  },
  preview: {
    port: 5173,
    proxy: apiProxy,
  },
  appType: "spa",
  test: {
    globals: true,
    environment: "happy-dom",
    setupFiles: ["./src/test-setup.ts"],
    include: ["src/**/*.test.{ts,tsx}"],
  },
})
