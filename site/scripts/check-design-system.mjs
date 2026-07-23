import { existsSync, readFileSync, readdirSync, statSync } from "node:fs";
import { dirname, extname, join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const siteRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const repositoryRoot = resolve(siteRoot, "..");
const designSystemRoot = join(repositoryRoot, "trop-design-system");
/** @type {string[]} */
const failures = [];

/** @param {string} path */
const relativeToRepository = (path) => relative(repositoryRoot, path);
/** @param {string} path */
const read = (path) => readFileSync(path, "utf8");

/**
 * @param {string} directory
 * @returns {string[]}
 */
function collectSourceFiles(directory) {
  return readdirSync(directory)
    .flatMap((entry) => {
      const path = join(directory, entry);
      return statSync(path).isDirectory() ? collectSourceFiles(path) : [path];
    })
    .filter((path) =>
      [".astro", ".css", ".js", ".mjs", ".ts"].includes(extname(path)),
    );
}

/**
 * @param {string} path
 * @param {string} reason
 */
function requireMissing(path, reason) {
  if (existsSync(path)) {
    failures.push(`${relativeToRepository(path)}: ${reason}`);
  }
}

/**
 * @param {string} path
 * @param {RegExp} pattern
 * @param {string} reason
 */
function requireText(path, pattern, reason) {
  if (!existsSync(path) || !pattern.test(read(path))) {
    failures.push(`${relativeToRepository(path)}: ${reason}`);
  }
}

requireText(
  join(designSystemRoot, "package.json"),
  /"name"\s*:\s*"@trop\/design-system"/,
  "the design system must remain an importable local package",
);
requireText(
  join(siteRoot, "package.json"),
  /"@trop\/design-system"\s*:\s*"file:\.\.\/trop-design-system"/,
  "declare the repository design system as a file dependency",
);

const landingPagePath = join(siteRoot, "src/pages/index.astro");
requireText(
  landingPagePath,
  /import "@trop\/design-system\/styles\.css"/,
  "import the global design-system entry point",
);
requireText(
  landingPagePath,
  /import "@trop\/design-system\/site\.css"/,
  "import the design-system site UI kit",
);
requireText(
  landingPagePath,
  /@trop\/design-system\/assets\/harbor-hero-light\.png\?url/,
  "load the light hero illustration from the design system",
);
requireText(
  landingPagePath,
  /@trop\/design-system\/assets\/harbor-hero-dark\.png\?url/,
  "load the dark hero illustration from the design system",
);
requireText(
  join(siteRoot, "astro.config.mjs"),
  /src:\s*"@trop\/design-system\/assets\/tool-mark\.svg"/,
  "load the Starlight logo from the design system",
);

for (const legacyPath of [
  "src/styles/theme.css",
  "src/styles/landing.css",
  "src/assets/tool-mark.svg",
  "public/favicon.svg",
  "public/assets/harbor-hero-light.png",
  "public/assets/harbor-hero-dark.png",
  "public/assets/harbor-backdrop.png",
  "public/assets/harbor-backdrop-dark.png",
]) {
  requireMissing(
    join(siteRoot, legacyPath),
    "legacy brand definitions and assets must not be duplicated in the site",
  );
}

for (const retiredAsset of [
  "assets/harbor-backdrop.png",
  "assets/harbor-backdrop-dark.png",
]) {
  requireMissing(
    join(designSystemRoot, retiredAsset),
    "retired hero artwork must not remain in the design system",
  );
}

const rawColor = /(?:#[\da-f]{3,8}\b|\b(?:rgb|hsl)a?\s*\()/gi;
const designTokenDeclaration =
  /--(?:tool|font|text|surface|border|accent|space|shadow|radius|duration|ease|leading|weight)-[\w-]+\s*:/g;

for (const path of collectSourceFiles(join(siteRoot, "src"))) {
  const source = read(path);
  for (const match of source.matchAll(rawColor)) {
    failures.push(
      `${relativeToRepository(path)}: raw color "${match[0]}"; use a design-system token`,
    );
  }
  for (const match of source.matchAll(designTokenDeclaration)) {
    failures.push(
      `${relativeToRepository(path)}: redeclares "${match[0].slice(0, -1)}"; define brand tokens in trop-design-system`,
    );
  }
}

const requiredComponents = [
  "Badge",
  "Button",
  "CommandCard",
  "CopyButton",
  "DocCard",
  "Eyebrow",
  "FeatureCard",
  "ScopePanel",
  "ThemeToggle",
];
const landingPage = read(landingPagePath);
for (const component of requiredComponents) {
  if (!landingPage.includes(`data-ds-component="${component}"`)) {
    failures.push(
      `${relativeToRepository(landingPagePath)}: missing ${component} design-system primitive`,
    );
  }
}

if (failures.length > 0) {
  console.error("Design-system adherence check failed:\n");
  for (const failure of failures) {
    console.error(`- ${failure}`);
  }
  process.exitCode = 1;
} else {
  console.log("Design-system adherence check passed.");
}
