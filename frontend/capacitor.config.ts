import type { CapacitorConfig } from "@capacitor/cli";

const config: CapacitorConfig = {
  appId: "com.nyxid.app",
  appName: "NyxID",
  webDir: "dist",
  plugins: {
    Keyboard: {
      resize: "body",
      resizeOnFullScreen: true,
    },
  },
  ios: {
    scheme: "NyxID",
    backgroundColor: "#06060A",
  },
};

// Dev: load from Vite dev server for live reload
// Prod: serve from bundled dist/ (no server block)
if (process.env.NODE_ENV !== "production") {
  config.server = {
    url: "http://localhost:5175",
    cleartext: true,
  };
}

export default config;
