import { describe, it, expect, vi, beforeEach } from "vitest";
import { apiClient, ApiError, api } from "./api-client";

const mockFetch = vi.fn();
vi.stubGlobal("fetch", mockFetch);

function jsonResponse(body: unknown, status = 200): Response {
  return {
    ok: status >= 200 && status < 300,
    status,
    json: () => Promise.resolve(body),
    headers: new Headers(),
  } as Response;
}

beforeEach(() => {
  mockFetch.mockReset();
});

describe("apiClient", () => {
  it("makes a GET request by default", async () => {
    mockFetch.mockResolvedValueOnce(jsonResponse({ id: "1" }));

    const result = await apiClient<{ id: string }>("/users/me");
    expect(result).toEqual({ id: "1" });
    expect(mockFetch).toHaveBeenCalledWith(
      "/api/v1/users/me",
      expect.objectContaining({ method: "GET", credentials: "include" }),
    );
  });

  it("sends JSON body for POST requests", async () => {
    mockFetch.mockResolvedValueOnce(jsonResponse({ success: true }));

    await apiClient("/auth/login", {
      method: "POST",
      body: { email: "test@test.com" },
    });

    const [, config] = mockFetch.mock.calls[0] as [string, RequestInit];
    expect(config.method).toBe("POST");
    expect(config.body).toBe('{"email":"test@test.com"}');
  });

  it("returns undefined for 204 status", async () => {
    mockFetch.mockResolvedValueOnce({
      ok: true,
      status: 204,
      json: () => Promise.reject(new Error("No body")),
      headers: new Headers(),
    } as Response);

    const result = await apiClient<void>("/auth/logout");
    expect(result).toBeUndefined();
  });

  it("throws ApiError on non-ok response", async () => {
    mockFetch.mockResolvedValueOnce({
      ok: false,
      status: 401,
      json: () =>
        Promise.resolve({
          error: "unauthorized",
          error_code: "1001",
          message: "Not authenticated",
        }),
      headers: new Headers(),
    } as Response);

    await expect(apiClient("/protected")).rejects.toThrow(ApiError);
  });

  it("ApiError contains status and error details", async () => {
    mockFetch.mockResolvedValueOnce({
      ok: false,
      status: 403,
      json: () =>
        Promise.resolve({
          error: "forbidden",
          error_code: "1002",
          message: "Access denied",
        }),
      headers: new Headers(),
    } as Response);

    try {
      await apiClient("/admin");
      expect.fail("should have thrown");
    } catch (err) {
      expect(err).toBeInstanceOf(ApiError);
      const apiErr = err as ApiError;
      expect(apiErr.status).toBe(403);
      expect(apiErr.errorCode).toBe("1002");
      expect(apiErr.message).toBe("Access denied");
    }
  });

  it("handles non-JSON error response", async () => {
    mockFetch.mockResolvedValueOnce({
      ok: false,
      status: 500,
      json: () => Promise.reject(new Error("Invalid JSON")),
      headers: new Headers(),
    } as Response);

    try {
      await apiClient("/broken");
      expect.fail("should have thrown");
    } catch (err) {
      expect(err).toBeInstanceOf(ApiError);
      const apiErr = err as ApiError;
      expect(apiErr.status).toBe(500);
      expect(apiErr.errorCode).toBe("UNKNOWN");
    }
  });

  it("does not send body when undefined", async () => {
    mockFetch.mockResolvedValueOnce(jsonResponse({ ok: true }));

    await apiClient("/endpoint");

    const [, config] = mockFetch.mock.calls[0] as [string, RequestInit];
    expect(config.body).toBeUndefined();
  });
});

describe("api convenience methods", () => {
  it("api.get makes GET request", async () => {
    mockFetch.mockResolvedValueOnce(jsonResponse({ data: "test" }));
    const result = await api.get<{ data: string }>("/test");
    expect(result.data).toBe("test");
  });

  it("api.post makes POST request", async () => {
    mockFetch.mockResolvedValueOnce(jsonResponse({ created: true }));
    await api.post("/items", { name: "item" });
    const [, config] = mockFetch.mock.calls[0] as [string, RequestInit];
    expect(config.method).toBe("POST");
  });

  it("api.put makes PUT request", async () => {
    mockFetch.mockResolvedValueOnce(jsonResponse({ updated: true }));
    await api.put("/items/1", { name: "updated" });
    const [, config] = mockFetch.mock.calls[0] as [string, RequestInit];
    expect(config.method).toBe("PUT");
  });

  it("api.patch makes PATCH request", async () => {
    mockFetch.mockResolvedValueOnce(jsonResponse({ patched: true }));
    await api.patch("/items/1", { name: "patched" });
    const [, config] = mockFetch.mock.calls[0] as [string, RequestInit];
    expect(config.method).toBe("PATCH");
  });

  it("api.delete makes DELETE request", async () => {
    mockFetch.mockResolvedValueOnce(jsonResponse({ deleted: true }));
    await api.delete("/items/1");
    const [, config] = mockFetch.mock.calls[0] as [string, RequestInit];
    expect(config.method).toBe("DELETE");
  });
});
