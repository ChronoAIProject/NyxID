import { StrictMode } from "react";
import { createRoot } from "react-dom/client";

import { CompanionApp } from "./companion-app";
import "./styles.css";

const root = document.getElementById("root");

if (!root) {
  throw new Error("Companion root element was not found");
}

createRoot(root).render(
  <StrictMode>
    <CompanionApp />
  </StrictMode>,
);
