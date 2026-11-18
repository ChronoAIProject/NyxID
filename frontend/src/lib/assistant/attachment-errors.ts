import { ApiError } from "@/lib/api-client";

export function attachmentFailure(error: unknown): string {
  if (error instanceof ApiError) {
    if (error.errorCode === 12101 || error.status === 410)
      return "This generated file has expired. Ask for a new result to generate it again.";
    if (error.status === 401 || error.status === 403)
      return "You are not authorized to read this attachment. Sign in as its owner.";
    if (error.status === 404)
      return "This attachment was not found in this conversation.";
  }
  return "The attachment could not be loaded. Try loading it again.";
}
