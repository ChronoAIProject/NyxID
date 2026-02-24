import React from "react";
import { createRoot } from "react-dom/client";
import { NyxIDProvider, createNyxClient } from "@nyxid/oauth-react";
import { App } from "./App";
import "./styles.css";

const baseUrl = import.meta.env.VITE_NYXID_BASE_URL;
const clientId = import.meta.env.VITE_NYXID_CLIENT_ID;
const redirectUri =
  import.meta.env.VITE_NYXID_REDIRECT_URI ||
  `${window.location.origin}/auth/callback`;

if (!baseUrl || !clientId) {
  throw new Error(
    "Missing env vars: VITE_NYXID_BASE_URL and VITE_NYXID_CLIENT_ID are required",
  );
}

if (clientId === "replace-with-your-client-id") {
  throw new Error(
    "VITE_NYXID_CLIENT_ID is still a placeholder. Create a NyxID OAuth client and put its id in demo-react/.env",
  );
}

const client = createNyxClient({
  baseUrl,
  clientId,
  redirectUri,
  scope: "openid profile email",
});

createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <NyxIDProvider client={client}>
      <App />
    </NyxIDProvider>
  </React.StrictMode>,
);
