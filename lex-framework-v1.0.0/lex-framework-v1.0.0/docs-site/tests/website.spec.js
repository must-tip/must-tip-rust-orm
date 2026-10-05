import { test, expect } from '@playwright/test';
import { docsNav } from '../src/docsManifest.js';

// Exercise the production bundle served by a static server beneath the GitHub Pages project prefix, including static-host hashes.
for (const [, slug] of docsNav.flatMap(group => group.items)) {
  test(`published documentation page ${slug} renders after a direct visit`, async ({ page }) => {
    const errors = [];
    page.on('pageerror', error => errors.push(error.message));
    await page.goto(`./#/docs/${slug}`);
    await expect(page.locator('.doc-article h1').first()).toBeVisible();
    await expect(page.getByRole('heading', { name: 'Page not found' })).toHaveCount(0);
    expect(errors).toEqual([]);
  });
}

test('a reader can navigate from the homepage and reload a documentation deep link', async ({ page }) => {
  await page.goto('./');
  await page.getByRole('link', { name: 'Start building' }).click();
  await expect(page).toHaveURL(/#\/docs\/quick-start$/);
  await page.reload();
  await expect(page.locator('.doc-article')).toBeVisible();
});

test('documentation search opens the selected result', async ({ page }) => {
  await page.goto('./');
  await page.getByRole('button', { name: 'Search docs' }).click();
  await page.getByPlaceholder('Search Lex v1.0.0 documentation').fill('Authentication');
  await page.locator('.search-results').getByRole('button', { name: 'Authentication', exact: false }).first().click();
  await expect(page).toHaveURL(/#\/docs\/authentication$/);
  await expect(page.locator('.doc-article')).toBeVisible();
});

test('table of contents scrolls without replacing the documentation route', async ({ page }) => {
  await page.goto('./#/docs/release-notes');
  await page.locator('.toc a').first().click();
  await expect(page).toHaveURL(/#\/docs\/release-notes$/);
  await expect(page.locator('.doc-article')).toBeVisible();
  await expect(page.locator('.doc-article h2').first()).toBeFocused();
});

test('theme preference survives a reload', async ({ page }) => {
  await page.goto('./');
  const initial = await page.locator('html').getAttribute('data-theme');
  await page.getByRole('button', { name: 'Toggle theme' }).click();
  await expect(page.locator('html')).toHaveAttribute('data-theme', initial === 'dark' ? 'light' : 'dark');
  await page.reload();
  await expect(page.locator('html')).toHaveAttribute('data-theme', initial === 'dark' ? 'light' : 'dark');
});

test('mobile navigation opens a documentation page', async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto('./#/docs/quick-start');
  await page.getByRole('button', { name: 'Open navigation' }).click();
  await page.locator('.drawer').getByRole('link', { name: 'Installation', exact: true }).click();
  await expect(page).toHaveURL(/#\/docs\/installation$/);
  await expect(page.locator('.doc-article')).toBeVisible();
});
