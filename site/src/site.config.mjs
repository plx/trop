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
    "tagline": "Steady localhost ports across directories and worktrees.",
    "description": "A small Rust CLI that keeps a local SQLite table of directory-to-port assignments so concurrent Git worktrees and local coding agents can restart dev servers without port collisions.",
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
        "label": "How it works",
        "href": "#model"
      },
      {
        "label": "Status",
        "href": "#status"
      },
      {
        "label": "GitHub",
        "href": "https://github.com/plx/trop"
      }
    ],
    "footerLinks": [
      {
        "label": "Overview",
        "href": "guides/overview/"
      },
      {
        "label": "Commands",
        "href": "guides/reference/"
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
      "label": "Read the guide",
      "href": "guides/overview/"
    },
    "secondaryCta": {
      "label": "Source on GitHub",
      "href": "https://github.com/plx/trop"
    },
    "terminal": {
      "title": "Example",
      "meta": "sh",
      "copy": "cargo install trop-cli",
      "note": "Prints only the port number on stdout, so it drops cleanly into shell scripts, Makefiles, or justfiles.",
      "lines": [
        "# Install from crates.io (or build from source)",
        "cargo install trop-cli",
        "",
        "# Ask for this directory's port, then pass it to your server",
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
            "label": "Overview & scope",
            "slug": "guides/overview"
          },
          {
            "label": "Common workflows",
            "slug": "guides/usage"
          },
          {
            "label": "Configuration",
            "slug": "guides/configuration"
          },
          {
            "label": "Commands & release notes",
            "slug": "guides/reference"
          }
        ]
      }
    ],
    "pages": [
      {
        "title": "Overview & scope",
        "description": "Why trop exists, how the local reservation table works, where its guarantees stop, and current project status.",
        "slug": "guides/overview",
        "href": "guides/overview/"
      },
      {
        "title": "Common workflows",
        "description": "Installing the CLI, reserving single or tagged ports, setting up trop.yaml groups, and cleaning up old entries.",
        "slug": "guides/usage",
        "href": "guides/usage/"
      },
      {
        "title": "Configuration",
        "description": "Port ranges, excluded ports, local overrides, occupancy probes, and how settings layer together.",
        "slug": "guides/configuration",
        "href": "guides/configuration/"
      },
      {
        "title": "Commands & release notes",
        "description": "Summary of all subcommands, exit codes, and upgrade notes for v0.2.0 and database schema v2.",
        "slug": "guides/reference",
        "href": "guides/reference/"
      }
    ]
  }
};
