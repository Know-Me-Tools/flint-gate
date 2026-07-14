import { test, expect, type Route } from '@playwright/test';

// ─────────────────────────────────────────────────────────────────────────────
// Approvals page smoke tests
//
// All tests use page.route() interceptors — no real admin server required.
// The polling test uses page.clock to fake timer advancement rather than
// waiting real seconds.
// ─────────────────────────────────────────────────────────────────────────────

const EMPTY_APPROVALS = { approvals: [] };

const ONE_APPROVAL = {
  approvals: [
    {
      approval_id: 'appr-test-001',
      principal_id: 'agent:researcher',
      action: 'call_tool',
      resource_id: 'tool:web_search',
      reason: 'Searching for test data',
      expires_at: new Date(Date.now() + 5 * 60 * 1000).toISOString(),
    },
  ],
};

/** Route all ancillary calls so the page renders without network errors. */
async function mockAncillary(route: Route) {
  const url = route.request().url();
  if (url.includes('/api/health')) {
    await route.fulfill({ json: { status: 'ok' } });
  } else if (url.includes('/api/events')) {
    await route.fulfill({
      status: 200,
      headers: { 'content-type': 'text/event-stream', 'cache-control': 'no-cache' },
      body: '',
    });
  } else {
    await route.continue();
  }
}

test.describe('Approvals page — empty state', () => {
  test.beforeEach(async ({ page }) => {
    // Register catch-all FIRST so specific routes (registered last) win in LIFO order.
    await page.route('**', mockAncillary);
    await page.route('/api/approvals', (r) => r.fulfill({ json: EMPTY_APPROVALS }));
  });

  test('renders "Pending Approvals" heading', async ({ page }) => {
    await page.goto('/approvals');
    await expect(page.getByRole('heading', { name: 'Pending Approvals' })).toBeVisible();
  });

  test('shows "No pending approvals." empty state', async ({ page }) => {
    await page.goto('/approvals');
    await expect(page.getByText('No pending approvals.')).toBeVisible();
  });
});

test.describe('Approvals page — table with pending approvals', () => {
  test.beforeEach(async ({ page }) => {
    await page.route('**', mockAncillary);
    await page.route('/api/approvals', (r) => r.fulfill({ json: ONE_APPROVAL }));
  });

  test('renders "Approval ID" column header', async ({ page }) => {
    await page.goto('/approvals');
    await expect(page.getByRole('columnheader', { name: 'Approval ID' })).toBeVisible();
  });

  test('renders the approval_id value in the table', async ({ page }) => {
    await page.goto('/approvals');
    await expect(page.getByText('appr-test-001')).toBeVisible();
  });

  test('renders the principal and action', async ({ page }) => {
    await page.goto('/approvals');
    await expect(page.getByText('agent:researcher')).toBeVisible();
    await expect(page.getByText('call_tool')).toBeVisible();
  });
});

test.describe('Approvals page — auto-polling via fake clock', () => {
  test('fires a second API call after 5 seconds have elapsed', async ({ page }) => {
    let callCount = 0;

    // Install fake clock BEFORE navigation so setInterval is captured immediately.
    await page.clock.install();

    await page.route('**', mockAncillary);
    await page.route('/api/approvals', async (r) => {
      callCount++;
      await r.fulfill({ json: EMPTY_APPROVALS });
    });

    await page.goto('/approvals');
    await expect(page.getByRole('heading', { name: 'Pending Approvals' })).toBeVisible();

    // Expect the initial fetch on mount.
    expect(callCount, 'expected initial fetch on mount').toBeGreaterThanOrEqual(1);
    const afterMount = callCount;

    // Advance fake clock past the 5-second interval.
    await page.clock.runFor(5_001);

    // At least one more call should have fired.
    expect(callCount, 'expected at least one poll after 5s').toBeGreaterThan(afterMount);
  });

  test('fires a third API call after 10 seconds have elapsed', async ({ page }) => {
    let callCount = 0;

    await page.clock.install();

    await page.route('**', mockAncillary);
    await page.route('/api/approvals', async (r) => {
      callCount++;
      await r.fulfill({ json: EMPTY_APPROVALS });
    });

    await page.goto('/approvals');
    await expect(page.getByRole('heading', { name: 'Pending Approvals' })).toBeVisible();

    await page.clock.runFor(5_001);
    await page.clock.runFor(5_001);

    // Initial + 2 interval ticks = at least 3 calls
    expect(callCount, 'expected ≥3 calls after two 5-second ticks').toBeGreaterThanOrEqual(3);
  });
});

test.describe('Approvals page — navigation', () => {
  test('is reachable from the root nav link', async ({ page }) => {
    await page.route('**', mockAncillary);
    await page.route('/api/approvals', (r) => r.fulfill({ json: EMPTY_APPROVALS }));
    await page.route('/api/routes', (r) => r.fulfill({ json: { routes: [], source: 'database' } }));
    await page.route('/api/api-keys', (r) => r.fulfill({ json: { keys: [] } }));
    await page.route('/api/policies', (r) => r.fulfill({ json: { policies: [] } }));
    await page.route('/api/tool-scopes', (r) => r.fulfill({ json: { tool_scopes: [] } }));

    await page.goto('/');
    await expect(page.getByRole('link', { name: 'Approvals' })).toBeVisible();
    await page.getByRole('link', { name: 'Approvals' }).click();
    await expect(page.getByRole('heading', { name: 'Pending Approvals' })).toBeVisible();
  });
});
