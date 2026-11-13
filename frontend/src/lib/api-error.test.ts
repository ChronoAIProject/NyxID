import { describe, expect, it } from "vitest";
import { apiErrorBody, responseApiError } from "./api-error";

const generic = "NyxID couldn't complete this request. Please try again.";

describe("supported error-envelope compatibility", () => {
  it("keeps OAuth descriptions and gives a NyxID message precedence", () => {
    const body = {
      error: "invalid_grant",
      error_description: "The authorization code expired.",
    };
    expect(apiErrorBody(body, 400)).toMatchObject({
      ...body,
      message: body.error_description,
    });
    expect(
      apiErrorBody(
        {
          ...body,
          message: "Explicit API message",
          error_code: 3000,
          details: { retry: true },
        },
        400,
      ),
    ).toMatchObject({
      message: "Explicit API message",
      error_code: 3000,
      details: { retry: true },
    });
  });

  it.each([
    [
      403,
      {
        error: "mfa_required",
        error_code: 2002,
        message: "MFA verification required",
        session_token: "synthetic-challenge",
      },
    ],
    [
      403,
      {
        error: "mfa_step_up_required",
        error_code: 2003,
        message: "Verify this session first",
        details: { reason: "mfa_step_up_required" },
      },
    ],
    [
      409,
      {
        error: "conflict",
        error_code: 1004,
        message: "Conflict: Request expired",
        details: { acknowledgement_id: "card", status: "expired" },
      },
    ],
  ])("keeps recovery fields for HTTP %s", async (status, body) => {
    const error = await responseApiError(Response.json(body, { status }));
    expect(error.message).toBe(body.message);
    expect(error.errorCode).toBe(body.error_code);
    expect(error.errorResponse).toEqual(body);
  });

  it.each([undefined, null])(
    "keeps numeric legacy envelopes with missing symbol %s",
    (error) => {
      const body = {
        error,
        error_code: 11300,
        message: "Insufficient credits",
        details: { owner: "self" },
      };
      expect(apiErrorBody(body, 402)).toMatchObject({
        error: "insufficient_credits",
        error_code: 11300,
        message: body.message,
        details: body.details,
      });
      // The same numeric code is ConnectLinkNotFound at 404, never a credits denial.
      expect(apiErrorBody(body, 404)).toMatchObject({
        error: "unknown_error",
        error_code: 11300,
        message: body.message,
        details: body.details,
      });
    },
  );

  it.each([
    { message: "private proxy message" },
    { error: "Bad gateway", message: "private proxy message" },
    { error_code: "1000", message: "private proxy message" },
    { error: { message: "private proxy message" } },
    { code: "bad_gateway", message: "private proxy message" },
  ])(
    "uses a generic message for unrecognized JSON instead of prose: %j",
    (body) => {
      expect(apiErrorBody(body, 400)).toEqual({
        error: "unknown_error",
        error_code: -1,
        message: generic,
      });
    },
  );
});

// Matches CMA layer1-public/rest/problems.rs and sdk/typescript/src/http.ts.
const cmaProblem = {
  type: "https://cma.example/problems/invalid-state",
  title: "invalid state",
  status: 409,
  detail: "The operation cannot run in the resource's current state.",
  code: "invalid_state",
  details: { resource: "agent", retryable: false },
};

it("requires a CMA adapter opt-in and retains the problem document fields", () => {
  expect(apiErrorBody(cmaProblem, 409).message).toBe(generic);
  expect(apiErrorBody(cmaProblem, 409, "cma")).toMatchObject({
    ...cmaProblem,
    error: cmaProblem.code,
    message: cmaProblem.detail,
  });
  expect(apiErrorBody(cmaProblem, 400, "cma").message).toBe(generic);
  expect(apiErrorBody({ code: "invalid_state" }, 409, "cma").message).toBe(
    generic,
  );
  expect(
    apiErrorBody({ title: "private proxy title" }, 409, "cma").message,
  ).toBe(generic);
});

it("keeps the CMA Responses typed error and rejects an incomplete nested object", () => {
  const error = {
    code: "invalid_request",
    message: "Unsupported input",
    type: "invalid_request_error",
    param: "input",
  };
  expect(apiErrorBody({ error }, 400, "cma")).toMatchObject({
    error: error.code,
    message: error.message,
    details: error,
  });
  expect(apiErrorBody({ error }, 400).message).toBe(generic);
  expect(
    apiErrorBody({ error: { message: "private proxy message" } }, 400, "cma")
      .message,
  ).toBe(generic);
});
