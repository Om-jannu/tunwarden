# Product

Tunwarden keeps tunnels running. It starts SSH forwards and any command that opens a local port (gcloud IAP, kubectl port-forward, AWS SSM and so on), checks that each one really listens, reconnects dropped ones, keeps a log per tunnel, and stops tunnels with a clear reason when a login expires, then brings them back after one re-login.

## Platform

adaptive

The CLI and daemon run on macOS, Windows and Linux. The desktop app (Tauri, a web view) runs on all three and lives mostly in the tray or menu bar. A website follows later at tunwarden.vercel.app.

## Users

- Primary: backend and DevOps engineers who reach databases, VMs and internal services through cloud or SSH tunnels every day and are comfortable in a terminal.
- Secondary: teammates and open-source users with similar setups who bring their own config.

## Product Purpose

Replace a pile of personal shell aliases and scripts with one reliable tool: start, stop and add tunnels, see real status, reconnect on its own, and make an expired login a one-step fix instead of a mystery.

## Positioning

Free, open source (MIT or Apache-2.0), local only. It supervises tools the user already has; it is not a VPN and not a replacement for ssh, gcloud or kubectl.

## Operating Context

Glanced at many times a day: a tray icon, a dense dashboard, or `tunwarden status` in a terminal. It should go unnoticed while things work and get attention only when a tunnel fails or a login expires. Production tunnels sit next to staging ones, so mistakes must be hard to make.

## Capabilities and Constraints

- Generic: no tool-specific code; tools are described in the user's config (profiles, logins, patterns).
- Safe by default: no stored credentials, commands never run through a shell, forwards bind to 127.0.0.1, production profiles ask before starting.
- Honest status: every failed tunnel carries a reason and a next step.
- No telemetry, no accounts, no network calls of its own.
- Distribution uses free services only, under the author's personal GitHub account (Om-jannu).

## Brand Commitments

- Name: Tunwarden.
- Logo (chosen 28 Nov 2026, concept B in `docs/brand/concepts/`): a pixel-art stone tunnel portal with a railway signal lamp inside. In the app icon the lamp is a warm neutral light; in the tray the lamp shows tunnel state (green running, amber connecting, red failed, unlit idle).
- Mascot: a pixel warden guarding the tunnel, shown on the website (and later first-run, empty states and installer art). Friendly (game-character) or serious (hooded keeper) reading is decided with the website design.
- Pixel art is drawn on exact grids (32 px icon, 16 pt tray) and only scaled by whole numbers.
- Where people work (dashboard, logs, terminal), the look stays plain and dense for scanning; brand expression goes to the icon, tray glyph, first-run and empty states, installers and the website.

## Evidence on Hand

None yet: no users, testimonials or benchmarks. Do not invent any.

## Product Principles

1. Quiet when healthy, clear when not.
2. Every problem says what happened and what to do next.
3. Nothing surprising: no shell, no hidden network, no silent changes to the user's tools.
4. The CLI and the app show the same truth from the same daemon.

## Accessibility & Inclusion

State is never shown by colour alone (text labels and icons accompany it). Light and dark follow the OS. The tray glyph must read at 16 px and work as a monochrome template image on macOS.
