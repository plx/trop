# Agent plugins

This repository is a marketplace for Claude Code and Codex. Each plugin packages
one `trop` skill: a short overview with references for installation, adoption,
groups, configuration, daily use, and validation. Install the `trop` executable
separately with `cargo install trop-cli`.

## Install

For **Codex**, add the marketplace and install the plugin:

```bash
codex plugin marketplace add plx/trop
codex plugin add trop@trop
```

Start a new thread and ask to use `trop`, or select the `trop` skill from the skill
picker. These commands require a Codex CLI with plugin marketplace support.
See the [Codex packaging documentation](https://developers.openai.com/plugins/build/plugins).

For **Claude Code**, run inside Claude Code:

```text
/plugin marketplace add plx/trop
/plugin install trop@trop
```

Describe a task such as “adopt trop for this project's development ports.” Claude
loads the reference skill when relevant; it is not a slash command. See
[Claude Code marketplaces](https://code.claude.com/docs/en/plugin-marketplaces).

To try an unmerged checkout, replace `plx/trop` in either marketplace-add command
with the checkout's absolute path. The GitHub commands above load the published
default branch, not local changes. Other agents supporting standalone Agent Skills
can use the complete `trop/skills/trop/` directory, including its references.

## Maintain

| Artifact | Purpose |
| --- | --- |
| `../.agents/plugins/marketplace.json` | Codex catalog; points to `./plugins/trop` from the repository root |
| `trop/.codex-plugin/plugin.json` | Codex plugin metadata and skill path |
| `trop/skills/trop/` | Canonical skill body, references, and Codex UI metadata |
| `../.claude-plugin/marketplace.json` | Claude catalog; points to `./plugins/claude/trop` |
| `claude/trop/` | Standalone Claude package with its own frontmatter and synchronized content |

Edit shared content in the canonical skill. Edit platform-specific frontmatter in
each edition's `SKILL.md`, then run:

```bash
just sync-plugin-skills
just validate-plugins
```

The sync script copies the body and references byte-for-byte, preserves Claude's
frontmatter, and excludes Codex's `agents/openai.yaml`. Claude uses separate
`description` and `when_to_use` fields with `user-invocable: false` for automatic
reference use. Codex retains its own discovery and UI metadata. See the
[Claude frontmatter reference](https://code.claude.com/docs/en/skills#frontmatter-reference).

Validation requires Python 3.9+, Just, Claude Code, and Codex. It checks copy drift
and package-local reference links, validates Claude manifests and skill files,
and ingests/installs the Codex package in temporary state. It leaves personal
plugin settings alone. CI pins the harness versions in `plugins.yml`.

For a local installed plugin, edit the source and reinstall `trop@trop`, then start
a new thread. Keep plugin versions in the two manifests aligned when releasing.
