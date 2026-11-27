// Facts the site shows about releases. Flip a flag once that part is on the
// GitHub release; until then the site points at building from source instead
// of links that would 404.
export const release = {
  version: "0.1.0",
  // The CLI release (release.yml, started by a `vX.Y.Z` tag).
  cli: false,
  // The desktop app (app-release.yml, started by hand for the same tag).
  app: false,
};

export const repo = "https://github.com/Om-jannu/tunwarden";
export const releases = `${repo}/releases`;
const latest = `${releases}/latest/download`;

export const install = {
  unix: `curl --proto '=https' --tlsv1.2 -LsSf ${latest}/tunwarden-cli-installer.sh | sh`,
  windows: `powershell -c "irm ${latest}/tunwarden-cli-installer.ps1 | iex"`,
  source: `cargo install --git ${repo} tunwarden-cli`,
  appSource: "pnpm install && pnpm desktop:build",
};

export type Os = "macos" | "windows" | "linux";

export const osName: Record<Os, string> = { macos: "macOS", windows: "Windows", linux: "Linux" };

// Tauri's default bundle names. Check them against the first app release.
const v = release.version;
export const appFiles: Record<Os, { label: string; file: string }[]> = {
  macos: [
    { label: "Apple silicon (.dmg)", file: `Tunwarden_${v}_aarch64.dmg` },
    { label: "Intel (.dmg)", file: `Tunwarden_${v}_x64.dmg` },
  ],
  windows: [
    { label: "Installer (.exe)", file: `Tunwarden_${v}_x64-setup.exe` },
    { label: "Windows Installer (.msi)", file: `Tunwarden_${v}_x64_en-US.msi` },
  ],
  linux: [
    { label: "Debian, Ubuntu (.deb)", file: `Tunwarden_${v}_amd64.deb` },
    { label: "Fedora, openSUSE (.rpm)", file: `Tunwarden-${v}-1.x86_64.rpm` },
    { label: "Any distribution (AppImage)", file: `Tunwarden_${v}_amd64.AppImage` },
  ],
};

export const appUrl = (file: string) => `${latest}/${file}`;

export function cliCommand(os: Os) {
  if (!release.cli) return install.source;
  return os === "windows" ? install.windows : install.unix;
}
