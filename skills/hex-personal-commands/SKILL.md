---
name: hex-personal-commands
description: Configure Agent-Desk personal deterministic transcript transformations in `~/.config/agent-desk/hex.config.ts`. Use when the user wants to add or change custom text transformations for the Agent-Desk voice app.
license: MIT
---

# Agent-Desk Personal Transformations

This skill targets current Agent-Desk (dev builds now, and the next stable release).
Agent-Desk 2.3.1 uses an older runtime; its matching skill installs with
`npx skills add anomalyco/hex#app-v2.3.1 --skill hex-personal-commands`.

Agent-Desk runs `~/.config/agent-desk/hex.config.ts` itself, in an embedded JavaScript
engine. There is no package to install and no build step.

1. Check for `~/.config/agent-desk/hex.config.ts` (or `hex.config.js`). If it is
   missing, ask the user to open Agent-Desk's Modes pane and choose Create Config,
   or run `agent-desk commands init` if the `agent-desk` CLI is available.
2. Read `~/.config/agent-desk/AGENTS.md` for the current API and limits, and
   `hex.d.ts` for exact types. Agent-Desk owns both and rewrites them; do not edit
   them.
3. Edit only the config and files it imports. Scripts are plain TypeScript or
   JavaScript: they can import `@hex/commands` and files in the folder, not
   npm packages, Effect, or Node APIs, and they have no filesystem, network,
   or timer access. Enums, namespaces, and constructor parameter properties are
   rejected.
4. Use `defineHexConfig` from `@hex/commands` and declare pure `transformations`
   entries (`{ id, name, description?, transform(text, context) }`).
5. Transformations must be selected in the mode's Transformations section.
   They run after mode corrections and optional AI rewriting, not for meetings.
6. Run `agent-desk commands check` (macOS) after editing. It loads the config as Agent-Desk
   does and prints what it registers or the error with file, line, and column.
   Optionally type-check with `npx -y -p typescript tsc` in `~/.config/agent-desk`. A
   running Agent-Desk reloads the config on save and shows load errors in Modes'
   Transformations section.
   If `agent-desk` reports `commands check` as an unrecognized subcommand, stop: the
   user is on an older Agent-Desk (such as 2.3.1). Tell them to install the matching
   skill with `npx skills add anomalyco/hex#app-v2.3.1 --skill hex-personal-commands`,
   or to switch to the dev build (https://github.com/anomalyco/hex#downloads).

Completion criterion: the requested behavior is represented in the config,
`agent-desk commands check` succeeds (or, without the CLI, the reload succeeded in
Agent-Desk), and any verification gap is reported.
