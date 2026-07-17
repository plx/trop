# ThemeToggle

The site's **light / dark / system** switch — a three-way segmented control.

## What it does

- Sets `data-theme="light"` or `data-theme="dark"` on `<html>`, or removes the
  attribute for **System** (defer to the OS `prefers-color-scheme`).
- Persists the choice in `localStorage` under `trop-theme`.
- On mount it re-applies the saved choice, so it stays in sync after reload.

The token layer (`tokens/colors.css`) does the rest: every `--tool-*` colour is
remapped by `[data-theme="dark"]` / the dark media query, so flipping the
attribute reskins the entire page uniformly — no dark islands in a light page.

## Boot snippet (avoid a flash)

Put this in the page `<head>`, before content renders, so the saved theme is
applied before first paint:

```html
<script>
  (function () {
    try {
      var m = localStorage.getItem("trop-theme");
      if (m === "light" || m === "dark") document.documentElement.setAttribute("data-theme", m);
    } catch (e) {}
  })();
</script>
```

## Usage

```jsx
import { ThemeToggle } from "./ThemeToggle.jsx";
// Drop it in the header actions, next to the CTA buttons.
<ThemeToggle />
```

## Notes

- Keep a **single** instance per page — multiple toggles won't observe each
  other's state.
- The hero engraving ships in **two theme-matched variants**:
  `harbor-backdrop.png` (day, blue-grey on cream) for light and
  `harbor-backdrop-dark.png` (harbor night) for dark. Swap the image per theme
  rather than hiding it (`[data-theme="dark"] .hero-art { background-image:
  url(.../harbor-backdrop-dark.png) }`); the `--surface-page` protection
  gradient blends into whichever theme is active.
