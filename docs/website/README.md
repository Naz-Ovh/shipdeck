# shipdeck docs website

The documentation site for shipdeck: a [Fumadocs](https://fumadocs.dev) app on
Next.js, exported as static files.

```sh
pnpm install
pnpm dev          # http://localhost:3000
pnpm build        # static site in out/
pnpm start        # serve out/ locally
pnpm types:check
```

`SITE_URL` in `.env.production` is the public address; `next build` uses it for
absolute links in social images and `llms.txt`.

## Deploy

The site runs on Cloudflare as a static-assets Worker (no Worker code) at
`shipdeck.naz.ovh`, configured in `wrangler.jsonc`.

```sh
pnpm exec wrangler login   # once
pnpm run deploy            # next build, then wrangler deploy
pnpm run preview           # next build, then serve out/ on Cloudflare's runtime locally
```

Use `pnpm run deploy`, not `pnpm deploy`: the latter is a built-in pnpm command.

- The `naz.ovh` zone must be on the Cloudflare account you log in with; the custom
  domain route creates the DNS record and certificate on the first deploy.
- `workers_dev` is off, so the site is only served on the custom domain.
- `public/_headers` sets long caching for `/_next/static/*`, UTF-8 content types for
  `llms.txt`, `llms-full.txt` and the per-page Markdown, and JSON for the search index.
- Unknown paths get `404.html` with a 404 status; `/docs/guides/ssh` serves `docs/guides/ssh.html`.
### On release

`.github/workflows/docs.yml` deploys the site when a GitHub Release is published
(`gh release create v0.3.1 --generate-notes`, or the Releases page). Pushing a tag
alone does not deploy. Pre-releases are skipped: the Release's pre-release box, or a
tag with a suffix such as `v0.4.0-beta.1`. The workflow checks out the release tag,
so the live docs match that release. "Run workflow" on the Actions tab redeploys by hand.

It needs two repository secrets (Settings → Secrets and variables → Actions):

| Secret | Value |
| --- | --- |
| `CLOUDFLARE_API_TOKEN` | A token from the "Edit Cloudflare Workers" template, with the `naz.ovh` zone included |
| `CLOUDFLARE_ACCOUNT_ID` | The account that owns the `naz.ovh` zone |

## LLM files

Every build writes `llms.txt` (an index linking to each page's Markdown),
`llms-full.txt` (every page, in sidebar order) and `llms.mdx/docs/<page>/content.md`.
`lib/llms.ts` turns the site's MDX components into plain Markdown for them.

## Layout

| Path | What it is |
| --- | --- |
| `content/docs/` | The pages, in MDX. `meta.json` files set the sidebar order and sections. |
| `app/(home)/` | The landing page. |
| `app/docs/` | The docs layout and page template. |
| `app/theme.css` | The Spruce palette: surface steps `sd-0` to `sd-5`, ink, jade accent, light and dark. |
| `app/surfaces.css` | Nested area coloring: each block inside another sits one shade deeper. |
| `app/chrome.css`, `app/prose.css`, `app/home.css` | Sidebar and TOC, article typography, landing page and terminal captures. |
| `components/screen.tsx` | `<Screen>`: a text capture of a shipdeck screen, framed like a terminal. |
| `components/pipeline-steps.tsx` | `<PipelineSteps />`: the ten deploy steps. |
| `app/fonts/` | JetBrains Mono files for box drawing and braille, which Google's subsets leave out (OFL). |

Fonts are Google Sans Flex and JetBrains Mono. The theme follows the system
light or dark setting, and the switch in the sidebar can override it.

## Writing pages

- Describe what the released version does; the code is the source of truth.
- Neutral examples only: `my-api`, `203.0.113.10`, user `deploy`, alias `prod`.
- Wrap `<id>`-style placeholders in backticks: bare `<` and `{` are JSX in MDX.
- Screen captures come from the TUI tests: `SHIPDECK_DUMP=/tmp/frames cargo test -p shipdeck`,
  then paste a `-100x30.txt` capture into `<Screen title="...">{`...`}</Screen>`.
