---
version: 1
slug: "src-app-page-tsx"
primary_target: "src/app/page.tsx"
related_targets: ["src/app/download/page.tsx","src/app/docs/page.tsx"]
---

# Website: landing, download, docs

Scope: tunwarden.vercel.app. Landing (`/`) is Persuade; download (`/download`) is Persuade with an Operate task (get the right file); docs (`/docs`) is Read.

- Audience: backend and DevOps engineers who reach databases and services through SSH and cloud tunnels.
- Job: understand in seconds that Tunwarden supervises the tunnel tools they already use, then install it.
- Action: Download (OS-detected) or copy the install command.
- Proof: real CLI output captured from tunwarden with demo tunnels (labelled), the real config format, the safety rules from PRODUCT.md, licence. No users, testimonials or numbers exist; none are invented.
- Constraints: static export on Vercel free tier; no analytics; release asset links must survive "no release yet".
- Unresolved: desktop app screenshots (needs the app run against a demo config while the user's own copy is closed).

## Direction contract

THESIS: The site is a signal box. Each tunnel is a track section on an illuminated diagram, its pixel portal lamp showing state; the lever frame under it works the demo. It refuses the split hero with a screenshot and three feature cards.

OWN-WORLD: Enamelled green-grey diagram panel (#2e3a35) set in a darker box (#1c2420), cream rails (#e9e6dc), black traffolyte plates with engraved cream capitals, red plates for danger, lamp aspects green #3fd77f, amber #ffbe3d, red #ff5a4c drawn as crisp pixel discs. Condensed engraved capitals for display and plates. The docs are the box's rule book on cream paper.

STORY: The visitor sees three tunnels on the diagram, pulls a lever to drop the network or expire a login, watches the lamp and plate say what happened and what to run, reads the real terminal transcript and box rules, then downloads.

FIRST VIEWPORT: The nameboard across the top with Download, Docs, GitHub plates. The diagram panel fills the viewport: headline "Every tunnel, signalled." and lede at left; three tracks run into pixel portals at right, each with lamp and plate (name, port, state in words); the warden stands by the portals with a speech plate. Under the panel, the lever frame: three demo levers, then the Download plate and the install command, the primary action at the bottom right of the fold.

FORM: Signal box (railway track diagram and lever frame), IMPECCABLE'S PICK, position 1 on my ordered list. seed:ec21b81dc764399a. Signature interaction: pulling a lever animates it through its throw and drives the diagram's state; motion is the lever throw plus the lamp changing aspect, nothing else.

FINISH: unreviewed and undocumented is unfinished; this build ends with the finish review, the verdict, DESIGN.md, and every shipping raster carrying its provenance
