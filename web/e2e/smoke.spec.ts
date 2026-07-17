import { test, expect } from '@playwright/test';

// These tests require a live admin server on port 4457.
// Skip them in CI where only the Vite dev server is available.
const skipWithoutServer = process.env.CI ? test.skip : () => {};

test.describe('Flint Gate Admin smoke tests', () => {
  test('loads the dashboard and navigation', async ({ page }) => {
    await page.goto('/');

    await expect(page.getByText('Flint Gate Admin')).toBeVisible();
    await expect(page.getByRole('link', { name: 'Dashboard' })).toBeVisible();
    await expect(page.getByRole('link', { name: 'Routes' })).toBeVisible();
    await expect(page.getByRole('link', { name: 'API Keys' })).toBeVisible();
  });

  test('navigates to Routes page and loads data from the admin API', async ({ page }) => {
    skipWithoutServer();
    await page.goto('/');
    await page.getByRole('link', { name: 'Routes' }).click();

    await expect(page.getByRole('heading', { name: 'Routes', exact: true })).toBeVisible();
    await expect(page.getByText('Manage proxy routes and their matching rules.')).toBeVisible();
    await expect(page.getByText('No routes configured.')).toBeVisible();
    await expect(page.getByText('0 route(s) from database')).toBeVisible();
  });

  test('navigates to API Keys page', async ({ page }) => {
    await page.goto('/');
    await page.getByRole('link', { name: 'API Keys' }).click();

    await expect(page.getByRole('heading', { name: 'API Keys' })).toBeVisible();
  });
});
