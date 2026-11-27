# Tunwarden website

The site at tunwarden.vercel.app: a landing page, a download page and short docs. Next.js with `output: "export"`, so `pnpm build` writes plain HTML to `out/`. No server, cookies or analytics.

```sh
pnpm install                         # at the repo root
pnpm --filter tunwarden-web dev      # http://localhost:3000
pnpm --filter tunwarden-web build    # static files in apps/web/out
```

## When a release goes out

`src/site.ts` has two flags. Until they are on, the site points at building from source instead of download links that would 404.

- `release.cli = true` once the `vX.Y.Z` release with the CLI installers is published.
- `release.app = true` once `app-release.yml` has added the desktop installers to it. Check the file names in `appFiles` against that release first; they are Tauri's defaults and include the version, so bump `release.version` with each release.

## Pixel art

The portal, lamp, tray glyph, warden and favicon come from `docs/brand/concepts/make.py`. After changing a drawing there, run `pnpm --filter tunwarden-web art` to regenerate `src/art/pixels.ts`. CI fails if the two disagree.

## Demo content

The terminal transcripts in `src/content/transcript.ts` were captured from `tunwarden` 0.1.0 with fake-tunnel processes and a stand-in login script; the page says so. The lever demo runs in the browser and uses the messages the daemon writes.

## Vercel (free Hobby plan)

1. Import the GitHub repository in Vercel.
2. Root Directory: `apps/web`. Framework preset: Next.js. Leave the build and output settings at their defaults (Vercel runs `pnpm install` at the workspace root and `next build`, and serves `out/`).
3. Environment variable `NEXT_TELEMETRY_DISABLED=1`.
4. Optional, to skip builds for commits that don't touch the site: Settings → Git → Ignored Build Step → `git diff --quiet HEAD^ HEAD -- . ../../docs/brand`.
5. Project name `tunwarden` gives `tunwarden.vercel.app` if the name is free.
