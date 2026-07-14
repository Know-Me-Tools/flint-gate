import { test, expect, type Route } from '@playwright/test';

// ─────────────────────────────────────────────────────────────────────────────
// Policies page smoke tests
//
// All tests use page.route() interceptors — no real admin server required.
// The Policies page also loads /api/tool-scopes and /api/health; mock those
// too so the page renders cleanly without network errors in the console.
// ─────────────────────────────────────────────────────────────────────────────

const POLICIES_FIXTURE = {
  policies: [
    {
      id: 'pol-alpha',
      policy_text: 'permit(principal, action, resource);',
      enabled: true,
      written_by: 'alice',
    },
    {
      id: 'pol-beta',
      policy_text: 'forbid(principal, action, resource);',
      enabled: true,
      written_by: null,
    },
  ],
};

const TOOL_SCOPES_FIXTURE = { tool_scopes: [] };

/** Build 20 version rows for pol-alpha starting from version_num `startAt` descending. */
function makeVersions(startAt: number, count: number) {
  return Array.from({ length: count }, (_, i) => ({
    id: startAt - i,
    policy_id: 'pol-alpha',
    version_num: startAt - i,
    policy_text: startAt - i === 1
      ? 'permit(principal, action, resource);'
      : `permit(principal, action, resource); // v${startAt - i}`,
    schema_json: null,
    entities_json: null,
    written_by: 'alice',
    written_at: new Date(Date.UTC(2026, 6, 1, 12, 0, 0) - i * 60_000).toISOString(),
  }));
}

/** Route all ancillary calls so the page renders without network errors. */
async function mockAncillary(route: Route) {
  const url = route.request().url();
  if (url.includes('/api/health')) {
    await route.fulfill({ json: { status: 'ok' } });
  } else if (url.includes('/api/tool-scopes')) {
    await route.fulfill({ json: TOOL_SCOPES_FIXTURE });
  } else if (url.includes('/api/events')) {
    // SSE stream — return an empty event-stream so the connection opens but
    // never emits (the banner stays hidden). Playwright keeps this open.
    await route.fulfill({
      status: 200,
      headers: { 'content-type': 'text/event-stream', 'cache-control': 'no-cache' },
      body: '',
    });
  } else {
    await route.continue();
  }
}

test.describe('Policies page — "Last by" column', () => {
  test.beforeEach(async ({ page }) => {
    await page.route('**', mockAncillary);
    await page.route('/api/policies', (r) => r.fulfill({ json: POLICIES_FIXTURE }));
    await page.goto('/policies');
    await expect(page.getByRole('heading', { name: 'Policies', level: 1 })).toBeVisible();
  });

  test('renders "Last by" column header', async ({ page }) => {
    await expect(page.getByRole('columnheader', { name: 'Last by' })).toBeVisible();
  });

  test('shows written_by value when present', async ({ page }) => {
    const alphaRow = page.locator('tr', { hasText: 'pol-alpha' });
    await expect(alphaRow.getByText('alice')).toBeVisible();
  });

  test('shows em-dash when written_by is null', async ({ page }) => {
    const betaRow = page.locator('tr', { hasText: 'pol-beta' });
    await expect(betaRow.getByText('—')).toBeVisible();
  });
});

test.describe('Policies page — history panel "Load more"', () => {
  // 20 versions on first fetch (total_hint: 25 → hasMore = true)
  const page1Versions = makeVersions(20, 20);
  // 5 more on second fetch (total_hint: 25 → hasMore = false after combining)
  const page2Versions = makeVersions(25, 5);

  test('shows "Load more" button when total_hint > versions returned', async ({ page }) => {
    await page.route('**', mockAncillary);
    await page.route('/api/policies', (r) => r.fulfill({ json: POLICIES_FIXTURE }));
    await page.route('/api/policies/pol-alpha/history*', (r) =>
      r.fulfill({
        json: {
          policy_id: 'pol-alpha',
          total_hint: 25,
          offset: 0,
          limit: 20,
          versions: page1Versions,
        },
      }),
    );

    await page.goto('/policies');
    await expect(page.getByRole('heading', { name: 'Policies', level: 1 })).toBeVisible();

    // Open edit modal for pol-alpha
    // The Actions cell has two icon buttons: Edit (first) and Delete (second)
    const alphaRow = page.locator('tr', { hasText: 'pol-alpha' });
    await alphaRow.getByRole('button').nth(0).click();
    await expect(page.getByRole('dialog')).toBeVisible();

    // Open the Version History accordion
    await page.getByRole('button', { name: /Version History/ }).click();

    // 20 version rows should be visible — spot-check first and last
    await expect(page.getByText('v20')).toBeVisible();
    // Use a cell locator to avoid partial-match against v10, v11, etc.
    await expect(page.getByRole('cell', { name: 'v1', exact: true })).toBeVisible();

    // "Load more" button should be present
    await expect(page.getByRole('button', { name: 'Load more' })).toBeVisible();
  });

  test('appends rows and hides "Load more" after loading all', async ({ page }) => {
    let historyCall = 0;
    await page.route('**', mockAncillary);
    await page.route('/api/policies', (r) => r.fulfill({ json: POLICIES_FIXTURE }));
    await page.route('/api/policies/pol-alpha/history*', (r) => {
      historyCall++;
      if (historyCall === 1) {
        return r.fulfill({
          json: {
            policy_id: 'pol-alpha',
            total_hint: 25,
            offset: 0,
            limit: 20,
            versions: page1Versions,
          },
        });
      }
      return r.fulfill({
        json: {
          policy_id: 'pol-alpha',
          total_hint: 25,
          offset: 20,
          limit: 20,
          versions: page2Versions,
        },
      });
    });

    await page.goto('/policies');
    await expect(page.getByRole('heading', { name: 'Policies', level: 1 })).toBeVisible();

    const alphaRow = page.locator('tr', { hasText: 'pol-alpha' });
    await alphaRow.getByRole('button').nth(0).click();
    await expect(page.getByRole('dialog')).toBeVisible();
    await page.getByRole('button', { name: /Version History/ }).click();

    await expect(page.getByRole('button', { name: 'Load more' })).toBeVisible();

    await page.getByRole('button', { name: 'Load more' }).click();

    // After loading: v25 should now appear (from page 2)
    await expect(page.getByText('v25')).toBeVisible();
    // "Load more" should be gone (25 total loaded)
    await expect(page.getByRole('button', { name: 'Load more' })).not.toBeVisible();
  });
});

test.describe('Policies page — diff toggle and PolicyDiffView', () => {
  const versions = [
    {
      id: 2,
      policy_id: 'pol-alpha',
      version_num: 2,
      policy_text: 'permit(principal, action, resource); // v2',
      schema_json: null,
      entities_json: null,
      written_by: 'alice',
      written_at: '2026-07-09T12:00:00Z',
    },
    {
      id: 1,
      policy_id: 'pol-alpha',
      version_num: 1,
      policy_text: 'permit(principal, action, resource);',
      schema_json: null,
      entities_json: null,
      written_by: 'alice',
      written_at: '2026-07-08T12:00:00Z',
    },
  ];

  test.beforeEach(async ({ page }) => {
    await page.route('**', mockAncillary);
    await page.route('/api/policies', (r) => r.fulfill({ json: POLICIES_FIXTURE }));
    await page.route('/api/policies/pol-alpha/history*', (r) =>
      r.fulfill({
        json: {
          policy_id: 'pol-alpha',
          total_hint: 2,
          offset: 0,
          limit: 20,
          versions,
        },
      }),
    );

    await page.goto('/policies');
    await expect(page.getByRole('heading', { name: 'Policies', level: 1 })).toBeVisible();

    const alphaRow = page.locator('tr', { hasText: 'pol-alpha' });
    await alphaRow.getByRole('button').nth(0).click();
    await expect(page.getByRole('dialog')).toBeVisible();
    await page.getByRole('button', { name: /Version History/ }).click();
    await expect(page.getByText('v2')).toBeVisible();
  });

  test('"Text / Diff" toggle is hidden for v1 (no prior version to diff against)', async ({ page }) => {
    // Click "View" on v1 row
    const v1Row = page.locator('tr', { hasText: 'v1' });
    await v1Row.getByRole('button', { name: 'View' }).click();

    // Toggle should NOT appear for v1
    await expect(page.getByRole('button', { name: 'Text' })).not.toBeVisible();
    await expect(page.getByRole('button', { name: 'Diff' })).not.toBeVisible();
  });

  test('"Text" and "Diff" toggle buttons are visible for v2', async ({ page }) => {
    const v2Row = page.locator('tr', { hasText: 'v2' });
    await v2Row.getByRole('button', { name: 'View' }).click();

    await expect(page.getByRole('button', { name: 'Text' })).toBeVisible();
    await expect(page.getByRole('button', { name: 'Diff' })).toBeVisible();
  });

  test('switching to "Diff" mode renders a <pre> with colored diff lines', async ({ page }) => {
    const v2Row = page.locator('tr', { hasText: 'v2' });
    await v2Row.getByRole('button', { name: 'View' }).click();

    await page.getByRole('button', { name: 'Diff' }).click();

    // The pre block should be visible
    const pre = page.locator('pre');
    await expect(pre).toBeVisible();

    // At least one line starting with '+' (not '+++') must appear — the green addition
    const content = await pre.textContent();
    expect(content).toBeTruthy();
    const hasAddedLine = (content ?? '').split('\n').some(
      (line) => line.startsWith('+') && !line.startsWith('+++'),
    );
    expect(hasAddedLine, 'expected at least one + diff line in output').toBe(true);
  });
});
