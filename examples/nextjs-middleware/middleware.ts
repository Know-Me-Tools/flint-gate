import { NextResponse } from "next/server";
import { createFlintGateMiddleware } from "@know-me/flint-gate";

const flintMiddleware = createFlintGateMiddleware({
  // Set this in production to the secret Flint Gate injects into
  // the x-flint-authenticated header.
  sharedSecret: process.env.FLINT_SHARED_SECRET,
  // Require the identity injected by Flint Gate to include these scopes.
  requiredScopes: ["chat"],
  // Skip static files and Next.js internals.
  bypassPrefixes: ["/_next", "/favicon.ico"],
});

export function middleware(request: Request) {
  const result = flintMiddleware(request);
  if (result instanceof Response) {
    return result;
  }

  // Request is authenticated. Optional: attach identity to headers for
  // downstream app code. In a real app you might store it in a request
  // context or short-lived cookie.
  const identity = result.identity;
  const reqHeaders = new Headers(request.headers);
  if (identity?.subject) {
    reqHeaders.set("x-downstream-user", identity.subject);
  }

  return NextResponse.next({
    request: { headers: reqHeaders },
  });
}

export const config = {
  matcher: ["/api/:path*", "/protected/:path*"],
};
