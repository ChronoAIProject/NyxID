import "./lib/zod-csp";
import "./lib/platform-personal-handoff";
import "./lib/platform-onboarding-handoff";
import { recoverFromAssetError } from "./lib/chunk-recovery";
import { isContentPath } from "./features/public-site/routes";
window.addEventListener("vite:preloadError", (event) => {
  if (recoverFromAssetError() === "reloading") event.preventDefault();
});
function bootFailed(error: unknown) {
  console.error(
    "[public-boot] Entry unavailable",
    error instanceof Error ? error.name : "error",
  );
  // Preserve any readable static body. A capped recovery failure remains actionable.
  const notice = document.createElement("div");
  notice.setAttribute("role", "alert");
  notice.style.cssText = "padding:24px;background:#18151e;color:#fff";
  notice.textContent = "NyxID could not load interactive controls. ";
  const retry = document.createElement("button");
  retry.textContent = "Retry";
  retry.onclick = () => window.location.reload();
  notice.append(retry);
  document.body.prepend(notice);
}
const path = window.location.pathname;
if (
  isContentPath(path) ||
  path.startsWith("/zh/") ||
  path.startsWith("/blog/") ||
  path.startsWith("/preview/")
) {
  void import("./features/public-site/bootstrap")
    .then((module) => module.bootPublicSite())
    .catch(bootFailed);
} else {
  void import("./app-entry").catch(bootFailed);
}
