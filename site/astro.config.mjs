import { defineConfig } from "astro/config";
import starlight from "@astrojs/starlight";
import { fileURLToPath } from "node:url";
import { siteConfig } from "./src/site.config.mjs";

const repositoryRoot = fileURLToPath(new URL("..", import.meta.url));

export default defineConfig({
  site: siteConfig.site.host,
  base: siteConfig.site.basePath,
  trailingSlash: "always",
  // Astro 7 changed the `compressHTML` default from `true` (HTML-aware) to
  // `'jsx'` (JSX-style), which strips whitespace between inline elements and can
  // drop rendered spaces. Pin the v6 behavior so the migration preserves the
  // deployed HTML output exactly.
  compressHTML: true,
  vite: {
    server: {
      fs: {
        allow: [repositoryRoot],
      },
    },
  },
  integrations: [
    starlight({
      title: siteConfig.project.title,
      description: siteConfig.project.description,
      logo: {
        src: "@trop/design-system/assets/tool-mark.svg",
        alt: "",
      },
      favicon: new URL(
        `${siteConfig.site.basePath}/favicon.svg`,
        siteConfig.site.host,
      ).href,
      customCss: [
        "@trop/design-system/styles.css",
        "./src/styles/starlight.css",
      ],
      social: [
        {
          icon: "github",
          label: "GitHub",
          href: siteConfig.repository.url,
        },
      ],
      editLink: {
        baseUrl: `${siteConfig.repository.url}/edit/${siteConfig.repository.defaultBranch}/site/src/content/docs/`,
      },
      sidebar: siteConfig.docs.sidebar,
    }),
  ],
});
