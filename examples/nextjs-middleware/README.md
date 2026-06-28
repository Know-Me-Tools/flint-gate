# Next.js Middleware Example

Uses the `@know-me/flint-gate` SDK middleware to validate that a request was authenticated by Flint Gate before it reaches Next.js routes.

## How it works

- `middleware.ts` runs on the Edge runtime.
- It checks for the `x-flint-authenticated` header injected by Flint Gate.
- If `FLINT_SHARED_SECRET` is set, the header value must match.
- It requires the `chat` scope in the `x-flint-identity` header.
- Static assets and `/_next/*` requests are bypassed.

## Setup

```bash
cd examples/nextjs-middleware
pnpm install
pnpm build-sdk
```

## Run

```bash
FLINT_SHARED_SECRET="s3cr3t" pnpm dev
```

Then proxy requests through Flint Gate, which should be configured to inject the authentication headers. Direct requests to `http://localhost:3000/api/hello` will fail with 401.

## Expected behavior

Request through Flint Gate with a valid identity that has scope `chat`:

```bash
curl -H "x-flint-authenticated: s3cr3t" \
     -H "x-flint-identity: %7B%22subject%22%3A%22user-1%22%2C%22scopes%22%3A%5B%22chat%22%5D%7D" \
     http://localhost:3000/api/hello
```

Response: `200 OK`

Request without the headers:

```bash
curl http://localhost:3000/api/hello
```

Response: `401 {"error":"missing flint-gate auth header"}`
