import type { IncomingMessage, ServerResponse } from "node:http";
import type { ViteDevServer } from "vite";
import { describe, expect, it, vi } from "vitest";
import { routingPreview } from "../../dev/routing-preview";

async function request(method: string, url: string) {
  const use = vi.fn();
  const plugin = routingPreview(
    "https://backend.example",
    "https://frontend.example",
  );
  if (typeof plugin.configureServer !== "function")
    throw new Error("Missing preview gateway");
  plugin.configureServer.call(
    {} as ThisParameterType<typeof plugin.configureServer>,
    {
      httpServer: { address: () => ({ port: 4317 }) },
      middlewares: { use },
    } as unknown as ViteDevServer,
  );
  const response = { writeHead: vi.fn(), end: vi.fn() };
  await use.mock.calls[0]![0](
    { method, url, headers: { host: "127.0.0.1:4317" } } as IncomingMessage,
    response as unknown as ServerResponse,
    vi.fn(),
  );
  return response.writeHead.mock.calls[0]?.[0];
}
const id = "00000000-0000-4000-8000-000000000001";
describe("pool metadata preview boundary", () => {
  it.each([
    "/service-pools",
    "/service-pools/candidates",
    `/service-pools/${id}`,
    `/service-pools/${id}/candidates`,
    `/service-pools/${id}/health?method=GET&path=/2/users/me`,
  ])("allows authenticated metadata reads of %s", async (path) => {
    // No session: admitted metadata requests reach the authentication gate.
    expect(await request("GET", `/api/v1${path}`)).toBe(401);
  });
  it.each([
    ["PUT", `/service-pools/${id}`],
    ["POST", "/service-pools"],
    ["POST", `/service-pools/${id}/health/reset`],
    ["GET", `/service-pools/${id}/health/reset`],
    ["GET", "/proxy/s/twitter-route"],
  ])("blocks %s %s", async (method, path) => {
    expect(await request(method!, `/api/v1${path}`)).toBe(403);
  });
});
