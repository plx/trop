**Button** — the primary action control. Filled harbor-green `primary`, outlined `secondary`/`ghost`; sizes `md` (44px) and `sm` (38px). Use it whenever the user takes an action or follows a CTA.

```jsx
<Button variant="primary" href="#install">Install trop</Button>
<Button variant="secondary" size="sm">View docs</Button>
<Button variant="ghost" icon={<GitHubIcon/>}>GitHub</Button>
```

Weight is always 800. Primary darkens on hover; outlined variants pick up a faint accent tint. Pass `href` to render an `<a>`.
