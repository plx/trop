**FeatureCard** — a bordered panel with a dark mono index chip, heading and body. Use in a 3-up grid to explain capabilities.

```jsx
<FeatureCard index="01" eyebrow="Idempotent" title="Same dir, same port">
  Repeated calls in the same worktree return the same reservation.
</FeatureCard>
```

Index chip is ink-on-surface, mono, weight 800. Omit `index` for a plain card.
