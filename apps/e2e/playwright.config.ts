// Playwright for the phone-size browser tests (tests/), run by scripts/e2e.sh (`make e2e`), which
// starts the server and the fake model server first and passes their address and the account.
// The browser is the system's Google Chrome (the `chrome` channel), so neither CI nor a developer
// downloads one; the viewport is a 390x844 phone with touch, where `(pointer: coarse)` holds.

import { defineConfig } from "@playwright/test";

export default defineConfig({
  testDir: "tests",
  timeout: 60_000,
  expect: { timeout: 15_000 },
  workers: 1,
  forbidOnly: Boolean(process.env.CI),
  reporter: process.env.CI ? [["list"], ["github"]] : "list",
  outputDir: process.env.E2E_OUTPUT ?? "test-results",
  use: {
    baseURL: process.env.E2E_BASE_URL,
    browserName: "chromium",
    channel: "chrome",
    viewport: { width: 390, height: 844 },
    deviceScaleFactor: 3,
    isMobile: true,
    hasTouch: true,
    userAgent:
      "Mozilla/5.0 (Linux; Android 16; Pixel 9) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/158.0.0.0 Mobile Safari/537.36",
    trace: "retain-on-failure",
    screenshot: "only-on-failure",
  },
});
