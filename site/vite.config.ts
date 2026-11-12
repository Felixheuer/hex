import react from "@vitejs/plugin-react"
import { defineConfig } from "vite"

// A static build for ai.sms-pilot.de, served by Caddy from /srv/pilot/site
// (docs/pilot-release.md). The SSR build only feeds the prerender.
export default defineConfig({
  plugins: [react()],
  base: "/",
})
