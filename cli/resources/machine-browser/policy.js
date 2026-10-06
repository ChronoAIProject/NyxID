/* Shared by the isolated content script and the extension service worker. */
const NyxIdFillerPolicy = Object.freeze({
  validate(request, now = Date.now()) {
    if (!request || typeof request !== "object") return "invalid_request";
    if (typeof request.nonce !== "string" ||
        !/^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/.test(request.nonce)) {
      return "invalid_nonce";
    }
    if (!Number.isSafeInteger(request.expires_at_ms) ||
        request.expires_at_ms <= now || request.expires_at_ms > now + 30000) {
      return "expired";
    }
    if (!["username", "password", "one_time_code"].includes(request.field)) return "wrong_field";
    if (!Array.isArray(request.allowed_origins) ||
        !request.allowed_origins.length || request.allowed_origins.length > 16) {
      return "origin_mismatch";
    }
    for (const origin of request.allowed_origins) {
      if (typeof origin !== "string" || origin.length > 2048) return "origin_mismatch";
      try {
        const url = new URL(origin);
        if (url.protocol !== "https:" || url.origin !== origin) return "origin_mismatch";
      } catch {
        return "origin_mismatch";
      }
    }
    return null;
  },
  suitable(field, type) {
    switch (field) {
      case "password": return type === "password";
      case "username": return ["text", "email", "tel"].includes(type);
      case "one_time_code": return ["text", "number", "tel"].includes(type);
      default: return false;
    }
  },
  valueAllowed(value) {
    return typeof value === "string" && value.length > 0 && new TextEncoder().encode(value).length <= 16384 &&
      !/[\0\r\n]/.test(value);
  },
});
