import { expect, it, vi } from "vitest";
import { readCmaJson } from "./cma-http";

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
