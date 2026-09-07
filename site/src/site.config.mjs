// Content and navigation for the Astro site. Visual foundations and component
// recipes belong in the local @trop/design-system package.
// prettier-ignore
export const siteConfig = {
  "repository": {
    "owner": "plx",
    "name": "trop",
    "url": "https://github.com/plx/trop",
    "defaultBranch": "main"
  },
  "project": {
    "name": "trop",
    "title": "trop",
    "packageName": "trop-site",
    "category": "Rust CLI",
    "tagline": "Port reservations for worktrees",
    "description": "trop assigns a port number to a directory and remembers it for the next run. Use it in dev scripts when you run several worktrees at once.",
    "installCommand": "cargo install trop-cli"
  },
  "site": {
    "host": "https://plx.github.io",
    "basePath": "/trop",
    "url": "https://plx.github.io/trop/",
    "dir": "site",
    "language": "en"
  },
  "landing": {
    "nav": [
      {
        "label": "Docs",
        "href": "guides/overview/"
      },
      {
        "label": "Reservations",
        "href": "#model"
      },
      {
        "label": "Install",
        "href": "#install"
      },
      {
        "label": "GitHub",
        "href": "https://github.com/plx/trop"
      }
    ],
    "footerLinks": [
      {
        "label": "Docs",
        "href": "guides/overview/"
      },
      {
        "label": "GitHub",
        "href": "https://github.com/plx/trop"
      },
      {
        "label": "License",
        "href": "https://github.com/plx/trop/blob/main/LICENSE"
      }
    ],
    "primaryCta": {
      "label": "Install",
      "href": "#install"
    },
    "secondaryCta": {
      "label": "Read docs",
      "href": "guides/overview/"
    },
    "terminal": {
      "title": "Install and use",
      "meta": "Bash / Zsh",
      "copy": "cargo install trop-cli",
      "lines": [
        "cargo install trop-cli",
        "",
        "PORT=$(trop reserve) &&",
        "  npm run dev -- --port \"$PORT\""
      ]
    }
  },
  "docs": {
    "sidebar": [
      {
        "label": "Guides",
        "items": [
          {
            "label": "Overview",
            "slug": "guides/overview"
          },
          {
            "label": "Usage",
            "slug": "guides/usage"
          },
          {
            "label": "Configuration",
            "slug": "guides/configuration"
          },
          {
            "label": "Scope",
            "slug": "guides/scope"
          }
        ]
      }
    ],
    "pages": [
      {
        "title": "Overview",
        "description": "How directory and tag pairs identify a reservation.",
        "slug": "guides/overview",
        "href": "guides/overview/"
      },
      {
        "title": "Usage",
        "description": "Install, reserve ports, and clean up old reservations.",
        "slug": "guides/usage",
        "href": "guides/usage/"
      },
      {
        "title": "Configuration",
        "description": "YAML settings, port ranges, exclusions, and defaults.",
        "slug": "guides/configuration",
        "href": "guides/configuration/"
      },
      {
        "title": "Scope",
        "description": "What a reservation guarantees, and what it cannot enforce.",
        "slug": "guides/scope",
        "href": "guides/scope/"
      }
    ]
  }
};
