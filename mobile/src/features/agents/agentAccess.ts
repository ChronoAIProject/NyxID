import { ApiError } from "../../lib/api/ApiError";

export function isAgentAccessRefusal(error: unknown) {
  return error instanceof ApiError && (error.statusCode === 403 || error.statusCode === 404);
}
