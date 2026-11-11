import { afterEach, expect, it, vi } from "vitest";
import { cmaJson, readCmaJson } from "./cma-http";

it("does not expose malformed successful response bodies in parser errors", async () => {
  const response = new Response("<html>private upstream page</html>", {
    headers: { "x-request-id": "request-2275" },
  });
  await expect(readCmaJson(response)).rejects.toMatchObject({
    status: 200,
    requestId: "request-2275",
    message: "NyxID couldn't complete this request. Please try again.",
  });
});

it("cancels oversized initial observations before parsing JSON", async () => {
  const cancel = vi.fn();
  const response = new Response(
    new ReadableStream({
      start(controller) {
        controller.enqueue(new TextEncoder().encode('{"text":"too large"}'));
      },
      cancel,
    }),
  );
  await expect(readCmaJson(response, 8)).rejects.toThrow(/limit/i);
  expect(cancel).toHaveBeenCalled();
});
it("accepts bounded UTF-8 JSON across chunk boundaries", async () => {
  const encoded = new TextEncoder().encode('{"text":"环境"}');
  const response = new Response(
    new ReadableStream({
      start(controller) {
        controller.enqueue(encoded.slice(0, 10));
        controller.enqueue(encoded.slice(10));
        controller.close();
      },
    }),
  );
  await expect(readCmaJson(response, 100)).resolves.toEqual({ text: "环境" });
});


afterEach(() => vi.unstubAllGlobals());

it.each([
  { status: 409, title: "invalid state", detail: "The operation cannot run in the resource's current state.", code: "invalid_state", details: { retryable: false } },
  { error: "sandbox_credential_setup_required", message: "Sign in to CMA", error_code: 42, details: { retryable: true } },
])("preserves CMA application refusals through cma-http", async (body) => {
  vi.stubGlobal("__nyxidAssistantHttpMock", () => Response.json(body, { status: 409 }));
  await expect(cmaJson("/assistant/cma/agents/agent")).rejects.toMatchObject({
    status: 409,
    message: "detail" in body ? body.detail : body.message,
    errorResponse: { error: "code" in body ? body.code : body.error, details: body.details },
  });
});
