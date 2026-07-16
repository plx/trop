const navToggle =
  document.querySelector<HTMLButtonElement>("[data-nav-toggle]");
const navPanel = document.querySelector<HTMLElement>("[data-nav-panel]");
const navLinks = document.querySelectorAll<HTMLElement>("[data-nav-link]");
const themeToggle = document.querySelector<HTMLElement>("[data-theme-toggle]");
const themeButtons =
  document.querySelectorAll<HTMLButtonElement>("[data-theme-mode]");

type ThemeMode = "system" | "light" | "dark";
const themeStorageKey = "trop-theme";

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

function setTheme(mode: ThemeMode): void {
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

setTheme(readTheme());

themeButtons.forEach((button) => {
  button.addEventListener("click", () => {
    const mode = button.dataset.themeMode;
    if (mode === "system" || mode === "light" || mode === "dark") {
      setTheme(mode);
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
