#!/usr/bin/env bun
// `bun run build`: release data, typecheck, client build (dist/), SSR build,
// then prerender. scripts/pilot-site-deploy.sh copies dist/ to /srv/pilot/site.
import { $ } from "bun"

await $`bun scripts/release-data.ts`
await $`tsc -b`
await $`vite build`
await $`vite build --ssr src/prerender.tsx --outDir dist-server`
await $`bun scripts/prerender.ts`
