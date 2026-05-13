document.querySelectorAll(".mq-icon-button[aria-pressed]").forEach((button) => {
  button.addEventListener("click", () => {
    const isPressed = button.getAttribute("aria-pressed") === "true";
    button.setAttribute("aria-pressed", String(!isPressed));
  });
});

document.querySelectorAll("[data-mq-nav]").forEach((header) => {
  const toggle = header.querySelector(".mq-site-header__toggle");
  const panel = header.querySelector(".mq-site-header__panel");

  if (!toggle || !panel) {
    return;
  }

  const setOpen = (open) => {
    toggle.setAttribute("aria-expanded", String(open));
    panel.dataset.open = String(open);
    panel.hidden = !open;
  };

  setOpen(false);

  toggle.addEventListener("click", () => {
    setOpen(toggle.getAttribute("aria-expanded") !== "true");
  });

  panel.querySelectorAll("a").forEach((link) => {
    link.addEventListener("click", () => setOpen(false));
  });
});

document.querySelectorAll(".mq-code-panel__copy").forEach((button) => {
  button.addEventListener("click", async () => {
    const panel = button.closest(".mq-code-panel");
    const body = panel?.querySelector(".mq-code-panel__body");
    const text = body?.textContent?.trim();

    if (!text || !navigator.clipboard) {
      return;
    }

    try {
      await navigator.clipboard.writeText(text);
      const original = button.textContent;
      button.textContent = "copied";
      window.setTimeout(() => {
        button.textContent = original;
      }, 1200);
    } catch {
      // Clipboard access is best-effort for static local previews.
    }
  });
});
