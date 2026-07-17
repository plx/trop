import AxeBuilder from "@axe-core/playwright";
import { expect, test } from "@playwright/test";

type DocsPage = {
  title: string;
  description: string;
  slug: string;
  href: string;
};

const origin = "http://127.0.0.1:4321";
const projectTitle = "trop";
const projectDescription =
  "A small CLI for stable localhost port numbers per worktree.";
const basePath: string = "/trop";
const normalizedBasePath = basePath === "/" ? "" : basePath;
// prettier-ignore
const docsPages: DocsPage[] = [
    {
      "title": "Overview",
      "description": "What trop reserves, why it exists, and where it fits.",
      "slug": "guides/overview",
      "href": "guides/overview/"
    },
    {
      "title": "Usage",
      "description": "Basic commands and script patterns for local port reservations.",
      "slug": "guides/usage",
      "href": "guides/usage/"
    },
    {
      "title": "Configuration",
      "description": "Port ranges, tags, exclusions, and cleanup behavior.",
      "slug": "guides/configuration",
      "href": "guides/configuration/"
    },
    {
      "title": "Scope",
      "description": "What trop deliberately does and does not attempt to solve.",
      "slug": "guides/scope",
      "href": "guides/scope/"
    }
  ];
const pagesToCheck = ["/", ...docsPages.map((page) => page.href)];
const pagesToAudit = ["/", docsPages[0]?.href].filter(Boolean);
const designSystemComponents = [
  "Badge",
  "Button",
  "CommandCard",
  "CopyButton",
  "DocCard",
  "Eyebrow",
  "FeatureCard",
  "ScopePanel",
  "ThemeToggle",
];

function sitePath(path = "/"): string {
  const cleanPath = path.startsWith("/") ? path : `/${path}`;
  return `${normalizedBasePath}${cleanPath}`;
}

function isSkippableHref(href: string): boolean {
  return (
    href === "" ||
    href.startsWith("mailto:") ||
    href.startsWith("tel:") ||
    href.startsWith("javascript:")
  );
}

test.describe("rendered site", () => {
  test("exposes core document and landmark properties", async ({ page }) => {
    await page.goto(sitePath("/"));

    expect(await page.title()).toContain(projectTitle);
    await expect(page.locator('meta[name="description"]')).toHaveAttribute(
      "content",
      projectDescription,
    );
    await expect(page.getByRole("main")).toBeVisible();
    await expect(
      page.getByRole("navigation", { name: /primary/i }),
    ).toBeVisible();
    await expect(
      page.getByRole("heading", { level: 1, name: projectTitle }),
    ).toBeVisible();
    await expect(page.locator(".skip-link")).toHaveAttribute("href", "#main");
  });

  test("keeps primary pages inside the viewport", async ({ page }) => {
    for (const pagePath of pagesToCheck) {
      await page.goto(sitePath(pagePath));
      await expect(page.getByRole("main")).toBeVisible();
      const hasHorizontalOverflow = await page.evaluate(
        () => document.documentElement.scrollWidth > window.innerWidth + 1,
      );
      expect(
        hasHorizontalOverflow,
        `${pagePath} should not overflow horizontally`,
      ).toBe(false);
    }
  });

  test("keeps the favicon inside the deployment base path", async ({
    page,
    request,
  }) => {
    for (const pagePath of ["/", docsPages[0]?.href].filter(Boolean)) {
      await page.goto(sitePath(pagePath));
      const faviconHref = await page
        .locator('link[rel~="icon"]')
        .getAttribute("href");

      expect(new URL(faviconHref!, origin).pathname).toBe(
        sitePath("/favicon.svg"),
      );
    }

    const response = await request.get(sitePath("/favicon.svg"));
    expect(response.status()).toBeLessThan(400);
  });

  test("preloads both theme-matched hero illustrations", async ({
    page,
    request,
  }) => {
    await page.goto(sitePath("/"));

    for (const theme of ["light", "dark"]) {
      const preload = page.locator(
        `link[rel="preload"][as="image"][data-theme-hero-preload="${theme}"]`,
      );
      await expect(preload).toHaveCount(1);
      await expect(preload).toHaveAttribute("fetchpriority", "high");

      const href = await preload.getAttribute("href");
      expect(href).toBeTruthy();
      const response = await request.get(href!);
      expect(response.status()).toBeLessThan(400);
    }
  });

  test("renders every landing primitive from the design-system contract", async ({
    page,
  }) => {
    await page.goto(sitePath("/"));

    for (const component of designSystemComponents) {
      await expect(
        page.locator(`[data-ds-component="${component}"]`),
        `${component} should be represented on the landing page`,
      ).not.toHaveCount(0);
    }

    const foundations = await page.evaluate(() => {
      const root = getComputedStyle(document.documentElement);
      const body = getComputedStyle(document.body);
      const command = getComputedStyle(
        document.querySelector<HTMLElement>(".command-card__body")!,
      );
      const docCard = getComputedStyle(
        document.querySelector<HTMLElement>(".doc-card")!,
      );

      return {
        accent: root.getPropertyValue("--tool-accent").trim(),
        bodyFont: body.fontFamily,
        commandFont: command.fontFamily,
        docCardShadow: docCard.boxShadow,
      };
    });

    expect(foundations.accent).toBe("#3a6b5f");
    expect(foundations.bodyFont).toContain("Source Sans 3");
    expect(foundations.commandFont).toContain("JetBrains Mono");
    expect(foundations.docCardShadow).toBe("none");
  });

  test("applies and persists the design-system theme modes", async ({
    page,
  }) => {
    await page.emulateMedia({ colorScheme: "light" });
    await page.goto(sitePath("/"));

    const themeGroup = page.getByRole("radiogroup", { name: "Color theme" });
    const system = themeGroup.getByRole("radio", { name: "System" });
    const light = themeGroup.getByRole("radio", { name: "Light" });
    const dark = themeGroup.getByRole("radio", { name: "Dark" });

    await expect(system).toHaveAttribute("aria-checked", "true");

    await dark.click();
    await expect(dark).toHaveAttribute("aria-checked", "true");
    await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
    await expect
      .poll(() =>
        page.evaluate(() => window.localStorage.getItem("trop-theme")),
      )
      .toBe("dark");

    const darkTheme = await page.evaluate(() => ({
      page: getComputedStyle(document.body).backgroundColor,
      terminal: getComputedStyle(
        document.querySelector<HTMLElement>(".command-card__body")!,
      ).backgroundColor,
      dayImage: getComputedStyle(
        document.querySelector<HTMLElement>(".hero")!,
        "::before",
      ).backgroundImage,
      dayOpacity: getComputedStyle(
        document.querySelector<HTMLElement>(".hero")!,
        "::before",
      ).opacity,
      nightImage: getComputedStyle(
        document.querySelector<HTMLElement>(".hero")!,
        "::after",
      ).backgroundImage,
      nightOpacity: getComputedStyle(
        document.querySelector<HTMLElement>(".hero")!,
        "::after",
      ).opacity,
    }));
    expect(darkTheme.page).toBe("rgb(12, 20, 33)");
    expect(darkTheme.terminal).toBe("rgb(10, 18, 32)");
    expect(darkTheme.dayImage).toContain("harbor-backdrop");
    expect(darkTheme.dayImage).not.toContain("harbor-backdrop-dark");
    expect(darkTheme.dayOpacity).toBe("0");
    expect(darkTheme.nightImage).toContain("harbor-backdrop-dark");
    expect(darkTheme.nightOpacity).toBe("0.96");

    await page.reload();
    await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
    await expect(dark).toHaveAttribute("aria-checked", "true");

    await light.click();
    await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
    await expect(light).toHaveAttribute("aria-checked", "true");

    await system.click();
    await expect(page.locator("html")).not.toHaveAttribute("data-theme", /.+/);
    await expect(system).toHaveAttribute("aria-checked", "true");
  });

  test("applies reduced-motion theme changes without waiting for hero decode", async ({
    page,
  }) => {
    await page.emulateMedia({ colorScheme: "light", reducedMotion: "reduce" });
    await page.addInitScript(() => {
      Object.defineProperty(HTMLImageElement.prototype, "decode", {
        configurable: true,
        value: () => new Promise<void>(() => {}),
      });
    });
    await page.goto(sitePath("/"));

    const dark = page
      .getByRole("radiogroup", { name: "Color theme" })
      .getByRole("radio", { name: "Dark" });
    await dark.click();

    await expect(dark).toHaveAttribute("aria-checked", "true");
    await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
    await expect
      .poll(() =>
        page.evaluate(() => window.localStorage.getItem("trop-theme")),
      )
      .toBe("dark");
  });

  test("copies the primary command through the design-system affordance", async ({
    context,
    page,
  }) => {
    await context.grantPermissions(["clipboard-read", "clipboard-write"]);
    await page.goto(sitePath("/"));

    const copyButton = page.getByRole("button", {
      name: "Copy command: cargo install trop-cli",
    });
    await copyButton.click();
    await expect(copyButton.locator("span:not(.sr-only)")).toHaveText("Copied");
    await expect
      .poll(() => page.evaluate(() => navigator.clipboard.readText()))
      .toBe("cargo install trop-cli");
  });

  test("manages the mobile navigation expanded state accessibly", async ({
    page,
  }) => {
    await page.goto(sitePath("/"));

    const toggle = page.locator("[data-nav-toggle]");
    if (!(await toggle.isVisible())) {
      return;
    }

    const panel = page.locator("[data-nav-panel]");
    await expect(toggle).toHaveAttribute("aria-controls", "mobile-nav");
    await expect(toggle).toHaveAttribute("aria-expanded", "false");
    await expect(panel).toBeHidden();

    await toggle.click();
    await expect(toggle).toHaveAttribute("aria-expanded", "true");
    await expect(panel).toBeVisible();

    await page.keyboard.press("Escape");
    await expect(toggle).toHaveAttribute("aria-expanded", "false");
    await expect(panel).toBeHidden();
  });

  test("validates rendered links and internal link targets", async ({
    page,
    request,
  }) => {
    const failures: string[] = [];

    for (const pagePath of pagesToCheck) {
      const response = await page.goto(sitePath(pagePath));
      expect(response?.status(), `${pagePath} should load`).toBeLessThan(400);

      const links = await page.locator("a[href]").evaluateAll((anchors) =>
        anchors.map((anchor) => ({
          href: anchor.getAttribute("href") ?? "",
          label: anchor.textContent?.trim() ?? "",
        })),
      );

      for (const link of links) {
        if (isSkippableHref(link.href)) {
          continue;
        }

        const resolved = new URL(link.href, `${origin}${sitePath(pagePath)}`);
        if (!["http:", "https:"].includes(resolved.protocol)) {
          failures.push(
            `${pagePath}: unsupported link protocol in ${link.href}`,
          );
          continue;
        }

        if (resolved.origin !== origin) {
          if (!link.label) {
            failures.push(
              `${pagePath}: external link ${link.href} has no text label`,
            );
          }
          continue;
        }

        if (
          normalizedBasePath &&
          resolved.pathname !== normalizedBasePath &&
          !resolved.pathname.startsWith(`${normalizedBasePath}/`)
        ) {
          failures.push(
            `${pagePath}: internal link escapes base path: ${link.href}`,
          );
          continue;
        }

        const targetPath = `${resolved.pathname}${resolved.search}`;
        const targetResponse = await request.get(targetPath);
        if (targetResponse.status() >= 400) {
          failures.push(
            `${pagePath}: ${link.href} returned ${targetResponse.status()}`,
          );
          continue;
        }

        if (resolved.hash) {
          await page.goto(`${targetPath}${resolved.hash}`);
          const targetExists = await page.evaluate((hash) => {
            const id = decodeURIComponent(hash.slice(1));
            return Boolean(
              document.getElementById(id) ||
              document.querySelector(`[name="${id}"]`),
            );
          }, resolved.hash);
          if (!targetExists) {
            failures.push(
              `${pagePath}: ${link.href} hash target does not exist`,
            );
          }
        }
      }
    }

    expect(failures).toEqual([]);
  });

  for (const pagePath of pagesToAudit) {
    test(`has no detectable accessibility violations on ${pagePath}`, async ({
      page,
    }) => {
      await page.goto(sitePath(pagePath));

      const results = await new AxeBuilder({ page }).analyze();
      expect(results.violations).toEqual([]);
    });
  }
});
