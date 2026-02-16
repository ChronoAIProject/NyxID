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

  it("throws ApiError on non-ok non-401 response", async () => {
    mockFetch.mockResolvedValueOnce({
      ok: false,
      status: 400,
      json: () =>
        Promise.resolve({
          error: "bad_request",
          error_code: "1000",
          message: "Invalid input",
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

describe("401 token refresh interceptor", () => {
  function errorResponse(status: number, errorCode: string, message: string): Response {
    return {
      ok: false,
      status,
      json: () =>
        Promise.resolve({
          error: "error",
          error_code: errorCode,
          message,
        }),
      headers: new Headers(),
    } as Response;
  }

  it("refreshes token and retries on 401", async () => {
    // 1st call: original request returns 401
    mockFetch.mockResolvedValueOnce(errorResponse(401, "1001", "Not authenticated"));
    // 2nd call: refresh endpoint succeeds
    mockFetch.mockResolvedValueOnce(jsonResponse({}, 200));
    // 3rd call: retried original request succeeds
    mockFetch.mockResolvedValueOnce(jsonResponse({ data: "success" }));

    const result = await apiClient<{ data: string }>("/users/me");

    expect(result).toEqual({ data: "success" });
    expect(mockFetch).toHaveBeenCalledTimes(3);
    // Verify refresh was called
    expect(mockFetch.mock.calls[1]?.[0]).toBe("/api/v1/auth/refresh");
    // Verify retry used same endpoint
    expect(mockFetch.mock.calls[2]?.[0]).toBe("/api/v1/users/me");
  });

  it("throws original 401 when refresh fails", async () => {
    // 1st call: original request returns 401
    mockFetch.mockResolvedValueOnce(errorResponse(401, "1001", "Not authenticated"));
    // 2nd call: refresh endpoint fails
    mockFetch.mockResolvedValueOnce(errorResponse(401, "1001", "Refresh failed"));

    try {
      await apiClient("/users/me");
      expect.fail("should have thrown");
    } catch (err) {
      expect(err).toBeInstanceOf(ApiError);
      const apiErr = err as ApiError;
      expect(apiErr.status).toBe(401);
      expect(apiErr.message).toBe("Not authenticated");
    }

    expect(mockFetch).toHaveBeenCalledTimes(2);
  });

  it("throws ApiError when retry after refresh also fails", async () => {
    // 1st call: original 401
    mockFetch.mockResolvedValueOnce(errorResponse(401, "1001", "Not authenticated"));
    // 2nd call: refresh succeeds
    mockFetch.mockResolvedValueOnce(jsonResponse({}, 200));
    // 3rd call: retry fails with 403
    mockFetch.mockResolvedValueOnce(errorResponse(403, "1002", "Forbidden"));

    try {
      await apiClient("/admin/users");
      expect.fail("should have thrown");
    } catch (err) {
      expect(err).toBeInstanceOf(ApiError);
      const apiErr = err as ApiError;
      expect(apiErr.status).toBe(403);
      expect(apiErr.message).toBe("Forbidden");
    }
  });

  it("returns undefined when retry yields 204", async () => {
    mockFetch.mockResolvedValueOnce(errorResponse(401, "1001", "Expired"));
    mockFetch.mockResolvedValueOnce(jsonResponse({}, 200));
    mockFetch.mockResolvedValueOnce({
      ok: true,
      status: 204,
      json: () => Promise.reject(new Error("No body")),
      headers: new Headers(),
    } as Response);

    const result = await apiClient<void>("/sessions/current");
    expect(result).toBeUndefined();
  });

  it("skips refresh for auth endpoints", async () => {
    const authEndpoints = [
      "/auth/login",
      "/auth/register",
      "/auth/refresh",
      "/auth/forgot-password",
      "/auth/reset-password",
      "/auth/verify-email",
      "/auth/setup",
    ];

    for (const endpoint of authEndpoints) {
      mockFetch.mockReset();
      mockFetch.mockResolvedValueOnce(errorResponse(401, "1001", "Unauthorized"));

      try {
        await apiClient(endpoint);
        expect.fail(`should have thrown for ${endpoint}`);
      } catch (err) {
        expect(err).toBeInstanceOf(ApiError);
        expect((err as ApiError).status).toBe(401);
      }

      // Only 1 call -- no refresh attempt
      expect(mockFetch).toHaveBeenCalledTimes(1);
    }
  });

  it("coalesces concurrent refresh attempts into a single call", async () => {
    // Set up responses for 2 concurrent requests that both get 401
    // Request 1: 401
    mockFetch.mockResolvedValueOnce(errorResponse(401, "1001", "Expired"));
    // Request 2: 401
    mockFetch.mockResolvedValueOnce(errorResponse(401, "1001", "Expired"));
    // Single shared refresh call
    mockFetch.mockResolvedValueOnce(jsonResponse({}, 200));
    // Request 1 retry
    mockFetch.mockResolvedValueOnce(jsonResponse({ id: "1" }));
    // Request 2 retry
    mockFetch.mockResolvedValueOnce(jsonResponse({ id: "2" }));

    const [r1, r2] = await Promise.all([
      apiClient<{ id: string }>("/users/1"),
      apiClient<{ id: string }>("/users/2"),
    ]);

    expect(r1).toEqual({ id: "1" });
    expect(r2).toEqual({ id: "2" });

    // 2 original + 1 refresh + 2 retries = 5 total
    expect(mockFetch).toHaveBeenCalledTimes(5);
    // Verify only 1 refresh call was made
    const refreshCalls = mockFetch.mock.calls.filter(
      (call: unknown[]) => call[0] === "/api/v1/auth/refresh",
    );
    expect(refreshCalls).toHaveLength(1);
  });

  it("handles network error during refresh gracefully", async () => {
    mockFetch.mockResolvedValueOnce(errorResponse(401, "1001", "Expired"));
    // Refresh call throws network error
    mockFetch.mockRejectedValueOnce(new TypeError("Failed to fetch"));

    try {
      await apiClient("/users/me");
      expect.fail("should have thrown");
    } catch (err) {
      expect(err).toBeInstanceOf(ApiError);
      expect((err as ApiError).status).toBe(401);
    }
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
