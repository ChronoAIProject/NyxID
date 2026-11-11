import { expect } from "@playwright/test";
import { randomUUID } from "node:crypto";
import { test } from "../e2e/production-policy-fixture";
import { mockDashboard } from "../e2e/managed-onboarding-fixtures";

test("desktop connection policy blocks companion ingress and permits the same-origin relay", async ({ page }, testInfo) => {
  const denied = "wss://companion-ingress.example/api/v1/policy-socket";
  testInfo.annotations.push({ type: "expected-csp", description: JSON.stringify({ directive: "connect-src", blocked: denied }) });
  await page.goto("/login");
  // Use real WebSockets here: Playwright's routeWebSocket bypasses CSP.
  const result = await page.evaluate(async (denied) => {
    const connect = (url: string) => new Promise<string>((resolve) => {
      const socket = new WebSocket(url);
      const timer = setTimeout(() => { socket.close(); resolve("timeout"); }, 5000);
      socket.onmessage = ({ data }) => { clearTimeout(timer); socket.close(); resolve(String(data)); };
      socket.onerror = () => { clearTimeout(timer); socket.close(); resolve("error"); };
    });
    return {
      companion: await connect(denied),
      browser: await connect(`${location.origin.replace(/^http/, "ws")}/api/v1/policy-socket`),
    };
  }, denied);
  expect(result).toEqual({ companion: "error", browser: "connected" });
});

test("human desktop streams through its own origin under enforcing CSP with separate node ingress", async ({ page }) => {
  await mockDashboard(page);
  await page.route("**/api/v1/public/config", (route) => route.fulfill({ json: {
    // This ingress is deliberately absent from the browser connect-src policy.
    node_ws_url: "wss://companion-ingress.example/api/v1/nodes/ws",
    version: "0.74.0",
  } }));
  await page.route("**/api/v1/assistant/nyxagent/machines/*/contexts", (route) =>
    route.fulfill({ json: [] }));
  const id = randomUUID();
  const session = randomUUID();
  const sockets: string[] = [];
  let send: ((data: Buffer) => void) | undefined;
  await page.routeWebSocket("**/api/v1/assistant/nyxagent/machines/*/desktop**", (socket) => {
    sockets.push(socket.url());
    send = (data) => socket.send(data);
    socket.send(JSON.stringify({ type: "connected", session_id: session, controller: "agent" }));
  });
  await page.goto(`/assistant/machines/${id}/desktop`);
  const viewer = page.getByRole("region", { name: "Live machine desktop", exact: true });
  await expect(viewer.getByRole("button", { name: "Take control" })).toBeEnabled();
  expect(sockets).toEqual([
    `${new URL(page.url()).origin.replace(/^http/, "ws")}/api/v1/assistant/nyxagent/machines/${id}/desktop`,
  ]);
  // Render synthetic pixels using the original companion frame format; no
  // screen capture or real node/browser credentials enter this fixture.
  const jpeg = await page.evaluate(() => {
    const canvas = document.createElement("canvas");
    canvas.width = 2;
    canvas.height = 2;
    const context = canvas.getContext("2d")!;
    context.fillStyle = "#ff0000";
    context.fillRect(0, 0, 2, 2);
    return canvas.toDataURL("image/jpeg").split(",")[1]!;
  });
  const header = Buffer.alloc(30);
  header.set([78, 89, 88, 77, 1, 0]);
  header.set(Buffer.from(session.replaceAll("-", ""), "hex"), 6);
  header.writeBigUInt64BE(1n, 22);
  send!(Buffer.concat([header, Buffer.from(jpeg, "base64")]));
  const canvas = viewer.getByLabel(/Machine screen/);
  await expect(canvas).toHaveAttribute("width", "2");
  await expect(canvas).toHaveAttribute("height", "2");
  const red = await canvas.evaluate((node) =>
    (node as HTMLCanvasElement).getContext("2d")!.getImageData(0, 0, 1, 1).data[0]);
  expect(red).toBeGreaterThan(240);
});
