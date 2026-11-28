"use client";

import { useEffect, useState } from "react";
import { appFiles, appUrl, install, osName, release, releases, type Os } from "@/site";
import { Command, useOs } from "./Install";

const order: Os[] = ["macos", "windows", "linux"];

const unsigned: Record<Os, string> = {
  macos: "The app is not notarised yet. Open it once, then go to System Settings → Privacy & Security and click Open Anyway.",
  windows: "The installer is not signed yet. If SmartScreen stops it, choose More info → Run anyway.",
  linux: "Install the .deb or .rpm with your package manager, or make the AppImage executable and run it.",
};

export function DownloadDesk() {
  const [detected, known] = useOs();
  const [os, setOs] = useState<Os>("macos");
  useEffect(() => setOs(detected), [detected]);

  return (
    <div className="desk">
      <div className="os-switch" role="tablist" aria-label="Operating system">
        {order.map((o) => (
          <button
            key={o}
            type="button"
            role="tab"
            id={`tab-${o}`}
            aria-selected={os === o}
            aria-controls="os-panel"
            onClick={() => setOs(o)}
          >
            {osName[o]}
            {known && o === detected ? <span className="os-yours"> · yours</span> : null}
          </button>
        ))}
      </div>

      <div id="os-panel" role="tabpanel" aria-labelledby={`tab-${os}`} className="desk-panel">
        <section className="desk-part" aria-labelledby="app-title">
          <h2 id="app-title">Desktop app</h2>
          <p className="desk-text">Tray or menu-bar app with the dashboard, logs and login flow. Includes the CLI.</p>
          {release.app ? (
            <ul className="files">
              {appFiles[os].map((f) => (
                <li key={f.file}>
                  <a className="file" href={appUrl(f.file)}>
                    <span className="file-label">{f.label}</span>
                    <span className="file-name">{f.file}</span>
                  </a>
                </li>
              ))}
            </ul>
          ) : (
            <>
              <p className="pending">
                <span className="plate">Not released yet</span> The first app release is being tested. Until then, build
                it from a clone of the repository (needs Rust and pnpm):
              </p>
              <Command text={install.appSource} label="Build the desktop app from source" />
            </>
          )}
          <p className="desk-text desk-small">{unsigned[os]}</p>
        </section>

        <section className="desk-part" aria-labelledby="cli-title">
          <h2 id="cli-title">CLI and daemon</h2>
          <p className="desk-text">
            For terminals, servers and scripts. The daemon runs in the background, so tunnels survive closing the
            terminal.
          </p>
          {release.cli ? (
            <Command
              text={os === "windows" ? install.windows : install.unix}
              label={`Install command for ${osName[os]}`}
            />
          ) : (
            <p className="pending">
              <span className="plate">Not released yet</span> Build it from source with Rust instead:
            </p>
          )}
          {release.cli ? <p className="desk-text desk-small">From source, on any platform with Rust:</p> : null}
          <Command text={install.source} label="Install from source with cargo" />
          {os === "macos" ? (
            <p className="desk-text desk-small">
              Prefer the install command to a browser download: archives downloaded in a browser are quarantined, and
              Gatekeeper blocks unsigned binaries.
            </p>
          ) : null}
        </section>
      </div>

      <p className="desk-text">
        Every file and the release notes are on the <a href={releases}>releases page</a>. Next:{" "}
        <a href="/docs/#quick-start">the quick start</a>.
      </p>
    </div>
  );
}
