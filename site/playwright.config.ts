import { defineConfig, devices } from "@playwright/test";

const basePath: string = "/trop";
const normalizedBasePath = basePath === "/" ? "" : basePath;
const localSiteUrl = `http://127.0.0.1:4321${normalizedBasePath}/`;
const dotReporter = ["dot"] as const;
const htmlReporter = ["html", { open: "never" }] as const;
const listReporter = ["list"] as const;

export default defineConfig({
  testDir: "./tests",
  fullyParallel: true,
  timeout: 30_000,
  expect: {
    timeout: 5_000,
  },
  reporter: process.env.CI
    ? [dotReporter, htmlReporter]
    : [listReporter, htmlReporter],
  use: {
    baseURL: "http://127.0.0.1:4321",
    trace: "on-first-retry",
  },
  webServer: {
    command: "npm run dev -- --host 127.0.0.1",
    url: localSiteUrl,
    // Astro 7 auto-detects AI-agent environments and starts `astro dev` in the
    // background, so the foreground process exits immediately and Playwright
    // reports "webServer exited early". Setting ASTRO_DEV_BACKGROUND opts out of
    // that detection and keeps the server in the foreground. This is a no-op in
    // CI (GitHub Actions is not detected as an agent), but lets the suite run
    // when launched from an agent shell.
    env: { ASTRO_DEV_BACKGROUND: "0" },
    reuseExistingServer: !process.env.CI,
    timeout: 120_000,
  },
  projects: [
    {
      name: "mobile",
      use: {
        browserName: "chromium",
        viewport: { width: 390, height: 844 },
        deviceScaleFactor: 3,
        isMobile: true,
        hasTouch: true,
      },
    },
    {
      name: "tablet",
      use: {
        browserName: "chromium",
        viewport: { width: 820, height: 1180 },
        deviceScaleFactor: 2,
        isMobile: true,
        hasTouch: true,
      },
    },
    {
      name: "desktop",
      use: {
        browserName: "chromium",
        ...devices["Desktop Chrome"],
        viewport: { width: 1440, height: 1000 },
      },
    },
  ],
});
