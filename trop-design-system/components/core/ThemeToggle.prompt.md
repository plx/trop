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
      if (m === "light" || m === "dark")
        document.documentElement.setAttribute("data-theme", m);
    } catch (e) {}
  })();
</script>
```

## Usage

```jsx
import { ThemeToggle } from "./ThemeToggle.jsx";
// Drop it in the header actions, next to the CTA buttons.
<ThemeToggle />;
```

## Notes

- Keep a **single** instance per page — multiple toggles won't observe each
  other's state.
- The hero engraving ships in **two theme-matched, compositionally aligned variants**:
  `harbor-hero-light.png` (day, blue-grey on cream) for light and
  `harbor-hero-dark.png` (harbor night) for dark. Preload both and keep them
  mounted as stacked layers, then cross-fade their opacity per theme. Do not
  swap a single layer's `background-image` URL: that can discover the other
  asset too late and produces a blank frame on the first transition. Give each
  layer its matching paper/night protection gradient.
