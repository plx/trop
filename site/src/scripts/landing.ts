const navToggle =
  document.querySelector<HTMLButtonElement>("[data-nav-toggle]");
const navPanel = document.querySelector<HTMLElement>("[data-nav-panel]");
const navLinks = document.querySelectorAll<HTMLElement>("[data-nav-link]");
const themeToggle = document.querySelector<HTMLElement>("[data-theme-toggle]");
const themeButtons =
  document.querySelectorAll<HTMLButtonElement>("[data-theme-mode]");

type ThemeMode = "system" | "light" | "dark";
type ResolvedTheme = Exclude<ThemeMode, "system">;
type ThemeViewTransition = {
  finished: Promise<void>;
};

const themeStorageKey = "trop-theme";
const themeTransitionFallbackMs = 760;
const heroImagePreloads = new Map<ResolvedTheme, Promise<void>>();
let themeAnimationId = 0;
let themeTransitionTimer: number | undefined;

document
  .querySelectorAll<HTMLLinkElement>("[data-theme-hero-preload]")
  .forEach((link) => {
    const theme = link.dataset.themeHeroPreload;
    if (theme !== "light" && theme !== "dark") {
      return;
    }

    const image = new Image();
    image.src = link.href;
    heroImagePreloads.set(
      theme,
      image.decode().catch(() => {
        // A failed preload must not prevent the reader from changing themes.
      }),
    );
  });

function readTheme(): ThemeMode {
  try {
    const storedTheme = window.localStorage.getItem(themeStorageKey);
    if (storedTheme === "light" || storedTheme === "dark") {
      return storedTheme;
    }
  } catch {
    // Storage can be unavailable in privacy-restricted browsing contexts.
  }

  return "system";
}

function applyTheme(mode: ThemeMode): void {
  if (mode === "system") {
    document.documentElement.removeAttribute("data-theme");
  } else {
    document.documentElement.dataset.theme = mode;
  }

  try {
    window.localStorage.setItem(themeStorageKey, mode);
  } catch {
    // The theme still applies for this page when persistence is unavailable.
  }

  themeButtons.forEach((button) => {
    button.setAttribute(
      "aria-checked",
      String(button.dataset.themeMode === mode),
    );
  });

  themeToggle?.setAttribute("data-theme-selection", mode);
}

function resolveTheme(mode: ThemeMode): ResolvedTheme {
  if (mode === "light" || mode === "dark") {
    return mode;
  }

  return window.matchMedia("(prefers-color-scheme: dark)").matches
    ? "dark"
    : "light";
}

function setTheme(mode: ThemeMode, animate = false): void {
  const root = document.documentElement;
  const reduceMotion = window.matchMedia(
    "(prefers-reduced-motion: reduce)",
  ).matches;

  if (!animate || reduceMotion) {
    applyTheme(mode);
    return;
  }

  const animationId = ++themeAnimationId;
  const startViewTransition = (
    document as Document & {
      startViewTransition?: (update: () => void) => ThemeViewTransition;
    }
  ).startViewTransition;

  if (startViewTransition) {
    root.setAttribute("data-theme-view-transition", "");

    try {
      const transition = startViewTransition.call(document, () => {
        applyTheme(mode);
      });
      const cleanup = () => {
        if (animationId === themeAnimationId) {
          root.removeAttribute("data-theme-view-transition");
        }
      };
      void transition.finished.then(cleanup, cleanup);
      return;
    } catch {
      root.removeAttribute("data-theme-view-transition");
    }
  }

  window.clearTimeout(themeTransitionTimer);
  root.setAttribute("data-theme-transition", "");
  // Flush the current theme with transitions enabled before changing tokens.
  void root.offsetWidth;
  applyTheme(mode);
  themeTransitionTimer = window.setTimeout(() => {
    if (animationId === themeAnimationId) {
      root.removeAttribute("data-theme-transition");
    }
  }, themeTransitionFallbackMs);
}

setTheme(readTheme());

themeButtons.forEach((button) => {
  button.addEventListener("click", async () => {
    const mode = button.dataset.themeMode;
    if (mode === "system" || mode === "light" || mode === "dark") {
      const requestId = ++themeAnimationId;
      await heroImagePreloads.get(resolveTheme(mode));
      if (requestId === themeAnimationId) {
        setTheme(mode, true);
      }
    }
  });
});

function setNavOpen(open: boolean): void {
  if (!navToggle || !navPanel) {
    return;
  }

  navToggle.setAttribute("aria-expanded", String(open));
  navToggle.setAttribute(
    "aria-label",
    open ? "Close navigation" : "Open navigation",
  );
  navPanel.hidden = !open;
  navPanel.dataset.open = String(open);
}

navToggle?.addEventListener("click", () => {
  setNavOpen(navToggle.getAttribute("aria-expanded") !== "true");
});

navLinks.forEach((link) => {
  link.addEventListener("click", () => setNavOpen(false));
});

document.addEventListener("keydown", (event) => {
  if (event.key === "Escape") {
    setNavOpen(false);
  }
});

async function copyText(text: string): Promise<void> {
  if (!navigator.clipboard) {
    throw new Error("Clipboard API is unavailable.");
  }

  await navigator.clipboard.writeText(text);
}

document
  .querySelectorAll<HTMLButtonElement>("[data-copy-text]")
  .forEach((button) => {
    let resetTimer: number | undefined;
    const visibleLabel =
      button.querySelector<HTMLElement>("span:not(.sr-only)");
    const status = button.querySelector<HTMLElement>("[data-copy-status]");
    const defaultVisibleText = visibleLabel?.textContent || "Copy";

    button.addEventListener("click", async () => {
      const text = button.dataset.copyText;
      if (!text) {
        return;
      }

      window.clearTimeout(resetTimer);
      try {
        await copyText(text);
        if (visibleLabel) {
          visibleLabel.textContent = "Copied";
        }
        if (status) {
          status.textContent = "Command copied to clipboard.";
        }
      } catch {
        if (visibleLabel) {
          visibleLabel.textContent = "Copy failed";
        }
        if (status) {
          status.textContent = "Copy failed.";
        }
      }

      resetTimer = window.setTimeout(() => {
        if (visibleLabel) {
          visibleLabel.textContent = defaultVisibleText;
        }
        if (status) {
          status.textContent = "";
        }
      }, 2200);
    });
  });
