import { defineConfig } from "astro/config";
import starlight from "@astrojs/starlight";

export default defineConfig({
  site: "https://plx.github.io",
  base: "/trop",
  trailingSlash: "always",
  integrations: [
    starlight({
      title: "trop",
      description: "Directory-aware localhost port reservations for worktrees.",
      logo: {
        src: "./src/assets/trop-mark.svg",
        alt: "trop",
      },
      social: [
        {
          icon: "github",
          label: "GitHub",
          href: "https://github.com/plx/trop",
        },
      ],
      customCss: ["./src/styles/starlight.css"],
      sidebar: [
        {
          label: "Guides",
          items: [
            { label: "Overview", slug: "guides/overview" },
            { label: "Usage", slug: "guides/usage" },
            { label: "Configuration", slug: "guides/configuration" },
            { label: "Scope", slug: "guides/scope" },
          ],
        },
      ],
    }),
  ],
});
