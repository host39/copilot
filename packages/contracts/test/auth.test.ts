// `@copilot/contracts` re-exports lib-rustauth's contracts, so the web app imports one package.
// Where both define a name (the envelopes, `errorResponseSchema`, `REQUEST_ID_HEADER`), Copilot's
// definition wins, so these tests prove the two still agree.

import {
  authRoutes,
  type DetailsResponse,
  type ErrorResponse,
  errorResponseSchema,
  hasRole,
  type ListResponse,
  REQUEST_ID_HEADER,
  registerRequestSchema,
  type SignInResponse,
  signInBodySchema,
  type UserRole,
} from "@contracts/index";
import { describe, expect, test } from "@rstest/core";
import * as auth from "@rustauth/contracts";

/** `true` only when `A` and `B` are the same type. */
type Equal<A, B> =
  (<T>() => T extends A ? 1 : 2) extends <T>() => T extends B ? 1 : 2 ? true : false;

// Compile-time checks: `make type-check` fails when the shared shapes drift apart.
const sameError: Equal<ErrorResponse, auth.ErrorResponse> = true;
const sameList: Equal<ListResponse<string>, auth.ListResponse<string>> = true;
const sameDetails: Equal<DetailsResponse<string>, auth.DetailsResponse<string>> = true;
const sameSignIn: Equal<SignInResponse, auth.SignInResponse> = true;
const sameRole: Equal<UserRole, auth.UserRole> = true;

describe("rustauth re-export", () => {
  test("shares the envelope types", () => {
    expect([sameError, sameList, sameDetails, sameSignIn, sameRole]).toEqual([
      true,
      true,
      true,
      true,
      true,
    ]);
  });

  test("passes rustauth's schemas and helpers through unchanged", () => {
    expect(signInBodySchema).toBe(auth.signInBodySchema);
    expect(registerRequestSchema).toBe(auth.registerRequestSchema);
    expect(authRoutes).toBe(auth.authRoutes);
    expect(hasRole("administrator", "manager")).toBe(true);
  });

  test("uses the same request id header", () => {
    expect(REQUEST_ID_HEADER).toBe(auth.REQUEST_ID_HEADER);
  });

  test("parses error bodies like rustauth", () => {
    const body = {
      error: "Too many failed sign-in attempts. Try again in 15 minutes.",
      code: "ACCOUNT_LOCKED",
      details: { retryAfterSecs: 900 },
      requestId: "c0ffee",
    };
    expect(errorResponseSchema.parse(body)).toEqual(auth.errorResponseSchema.parse(body));
    expect(auth.retryAfterSecs(errorResponseSchema.parse(body))).toBe(900);
  });
});
