import { test, expect, request as pwRequest } from '@playwright/test';

// ─────────────────────────────────────────────────────────────────────────────
// OAuth surface E2E against a real Ory Hydra (docker-compose.smoke.yml).
// These hit the gateway's HTTP API directly (not the admin UI), so they use the
// Playwright `request` fixture with absolute URLs rather than the UI baseURL.
//
// Requires the smoke stack up with Hydra seeded (flint-e2e-client). Skipped
// automatically when the gateway/Hydra are not reachable, so a UI-only smoke run
// does not fail. Run explicitly:
//   E2E_OAUTH=1 pnpm --dir web playwright test oauth.spec.ts
// ─────────────────────────────────────────────────────────────────────────────

// Ports exposed on the host by the smoke compose.
const GATEWAY = process.env.GATEWAY_URL ?? 'http://localhost:4456';
const HYDRA_PUBLIC = process.env.HYDRA_PUBLIC_URL ?? 'http://localhost:4444';
const CLIENT_ID = 'flint-e2e-client';
const CLIENT_SECRET = 'flint-e2e-secret';

const oauthEnabled = process.env.E2E_OAUTH === '1';

/** Mint a real client-credentials access token straight from Hydra (the subject). */
async function hydraClientCredentialsToken(): Promise<string> {
  const ctx = await pwRequest.newContext();
  const res = await ctx.post(`${HYDRA_PUBLIC}/oauth2/token`, {
    form: {
      grant_type: 'client_credentials',
      client_id: CLIENT_ID,
      client_secret: CLIENT_SECRET,
      scope: '',
    },
  });
  expect(res.ok(), `hydra token mint failed: ${res.status()}`).toBeTruthy();
  const body = await res.json();
  await ctx.dispose();
  return body.access_token as string;
}

test.describe('OAuth surface — happy path (real Ory Hydra)', () => {
  test.skip(!oauthEnabled, 'set E2E_OAUTH=1 with the smoke stack up to run OAuth E2E');

  test('client obtains a Hydra access token (subject bootstrap)', async () => {
    const token = await hydraClientCredentialsToken();
    expect(token).toBeTruthy();
    // A JWT access token has three dot-separated segments.
    expect(token.split('.').length).toBe(3);
  });

  test('RFC 8693 token exchange delegates to Hydra and returns a Hydra-minted token', async ({
    request,
  }) => {
    const subject = await hydraClientCredentialsToken();
    const res = await request.post(`${GATEWAY}/oauth/token`, {
      form: {
        grant_type: 'urn:ietf:params:oauth:grant-type:token-exchange',
        subject_token: subject,
        subject_token_type: 'urn:ietf:params:oauth:token-type:access_token',
      },
    });
    expect(res.ok(), `exchange failed: ${res.status()}`).toBeTruthy();
    const body = await res.json();
    // The gateway relays Hydra's minted token verbatim (delegate mode).
    expect(body.access_token, 'no access_token in exchange response').toBeTruthy();
  });

  test('RFC 7662 introspection reports an active Hydra token (authenticated)', async ({
    request,
  }) => {
    const subject = await hydraClientCredentialsToken();
    const res = await request.post(`${GATEWAY}/oauth/introspect`, {
      // Client auth on the introspection endpoint (RFC 7662 §2.1).
      form: {
        token: subject,
        client_id: CLIENT_ID,
        client_secret: CLIENT_SECRET,
      },
    });
    expect(res.ok(), `introspect failed: ${res.status()}`).toBeTruthy();
    const body = await res.json();
    expect(body.active, `expected active token, got ${JSON.stringify(body)}`).toBe(true);
  });
});

// NOTE: the Hydra transport / non-2xx / redirect deny paths are covered
// deterministically by the Rust unit tests (token_exchange.rs / introspect.rs
// `delegate_fails_closed_on_*`, incl. the no-redirect 302 guard). Reproducing a
// live Hydra outage inside the compose network would be flaky and slow, so those
// are intentionally not re-run here — the E2E covers the request-side denials
// (unauth, over-rate, actor_token) against the real stack.
test.describe('OAuth surface — fail-closed denials (real Ory Hydra)', () => {
  test.skip(!oauthEnabled, 'set E2E_OAUTH=1 with the smoke stack up to run OAuth E2E');

  test('unauthenticated introspection is rejected (RFC 7662 §2.1 → 401)', async ({ request }) => {
    // No client_id/client_secret and no Basic header → the endpoint must refuse
    // before doing any token scanning.
    const res = await request.post(`${GATEWAY}/oauth/introspect`, {
      form: { token: 'anything' },
    });
    expect(res.status(), 'unauth introspect must be 401').toBe(401);
  });

  test('a present actor_token is rejected (single-hop only → 400 invalid_request)', async ({
    request,
  }) => {
    const subject = await hydraClientCredentialsToken();
    const res = await request.post(`${GATEWAY}/oauth/token`, {
      form: {
        grant_type: 'urn:ietf:params:oauth:grant-type:token-exchange',
        subject_token: subject,
        subject_token_type: 'urn:ietf:params:oauth:token-type:access_token',
        actor_token: subject, // multi-hop delegation is not supported
        actor_token_type: 'urn:ietf:params:oauth:token-type:access_token',
      },
    });
    expect(res.status(), 'actor_token must be 400').toBe(400);
    const body = await res.json();
    expect(body.error).toBe('invalid_request');
  });

  test('over-rate requests are throttled (429)', async ({ request }) => {
    // The smoke config sets oauth.rate_limit per_second=5 / burst=10 (in-process
    // governor; no Redis L2 in the smoke stack). A rapid unauth burst to the
    // introspection endpoint must produce at least one 429. Sent concurrently so
    // the burst bucket is exhausted deterministically rather than on a timer.
    const attempts = 60;
    const results = await Promise.all(
      Array.from({ length: attempts }, () =>
        request
          .post(`${GATEWAY}/oauth/introspect`, { form: { token: 'x' } })
          .then((r) => r.status()),
      ),
    );
    expect(
      results.some((s) => s === 429),
      `expected at least one 429 in ${attempts} rapid requests, got ${JSON.stringify(
        [...new Set(results)],
      )}`,
    ).toBeTruthy();
  });
});
