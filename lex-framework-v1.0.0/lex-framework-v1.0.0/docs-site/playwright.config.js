import { defineConfig, devices } from '@playwright/test';

export default defineConfig({
  testDir: './tests',
  forbidOnly: !!process.env.CI,
  retries: 0,
  workers: process.env.CI ? 2 : undefined,
  reporter: [['list']],
  use: {
    baseURL: 'http://127.0.0.1:4187/must-tip-rust-orm/',
    trace: 'retain-on-failure',
    screenshot: 'only-on-failure',
  },
  projects: [{ name: 'chromium', use: { ...devices['Desktop Chrome'] } }],
  webServer: {
    command: 'python3 ../integration/serve_docs.py --port 4187',
    url: 'http://127.0.0.1:4187/must-tip-rust-orm/',
    reuseExistingServer: false,
    timeout: 30000,
  },
});
