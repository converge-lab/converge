import { test, expect } from '@playwright/test';
import checkUiErrors from './check-ui-errors.js';

test('API errors remain visible and safe across the UI', async ({ page }) => {
  const result = await checkUiErrors(page);
  expect(result.passed).toBeGreaterThanOrEqual(35);
});
