# Flint Gate Docs

This directory contains the Docusaurus documentation site for [Flint Gate](https://github.com/know-me-tools/flint-gate).

## Build

Install dependencies:

```bash
npm install
```

Start the development server:

```bash
npm run start
```

Build the static site:

```bash
npm run build
```

## Deploy to GitHub Pages

The site is configured for `https://know-me-tools.github.io/flint-gate/`.

```bash
npm run deploy
```

This uses the Docusaurus GitHub Pages deployer and pushes the `build/` directory to the `gh-pages` branch.

## Structure

- `docs/` — Markdown documentation pages
- `src/pages/index.tsx` — Landing page
- `src/css/custom.css` — Theme overrides
- `static/openapi.json` — Admin API OpenAPI spec
- `docusaurus.config.ts` — Site configuration
- `sidebars.ts` — Generated sidebar layout
