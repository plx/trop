**CommandCard** — trop's hero terminal panel. Titled header with mono meta, a
theme-matched code body and mono text, a faint dual-corner accent wash, and an
optional copy button. Use for install snippets and command examples.

```jsx
<CommandCard title="Reserve a port" meta="bash"
  code={`PORT=$(trop reserve)\nnpm run dev -- --port "$PORT"`}
  copyText='PORT=$(trop reserve)' />
```

Use `compact` for short one-liners. Pass multiline strings (or children) for the body.
