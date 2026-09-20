import { defineConfig } from '@playwright/test';
import { existsSync } from 'node:fs';

const chrome = '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome';
const executablePath = process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE_PATH ||
  (!process.env.CI && existsSync(chrome) ? chrome : undefined);

export default defineConfig({
  testDir: './scripts',
  testMatch: '*.spec.js',
  workers: 1,
  timeout: 60_000,
  outputDir: './target/playwright',
  reporter: 'list',
  use: {
    baseURL: 'http://127.0.0.1:8086',
    viewport: { width: 1280, height: 900 },
    launchOptions: { executablePath },
    trace: 'retain-on-failure',
    screenshot: 'only-on-failure',
  },
  webServer: {
    command: 'python3 -m http.server 8086 --bind 127.0.0.1 --directory crates/converge-web/dist',
    url: 'http://127.0.0.1:8086',
    reuseExistingServer: false,
    stderr: 'ignore',
  },
});
