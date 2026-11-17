/**
 * Some macOS hosts cannot launch native Firefox:
 * sandbox_extension_issue_file_to_process failed: 1 (Operation not permitted).
 * Local-only substitute used with Playwright 1.62.0 (runner/Vite stay on host):
 * docker run --rm --init --name nyxid-2451-firefox-iter4 -p 127.0.0.1:4661:4661 mcr.microsoft.com/playwright:v1.62.0-noble npx -y playwright@1.62.0 run-server --port 4661 --host 0.0.0.0
 * In a temporary UNTRACKED local config, select only the Firefox specs and set
 * use.connectOptions = { wsEndpoint: "ws://127.0.0.1:4661/", exposeNetwork: "<loopback>" }.
 * Keep that config untracked; delete it and stop the container after the run.
 */
import { test } from "@playwright/test";
import { accountIsolationCases } from "./account-isolation.cases";

test.use({ browserName: "firefox" });
accountIsolationCases();
