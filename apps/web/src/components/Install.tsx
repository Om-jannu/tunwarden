"use client";

import Link from "next/link";
import { useEffect, useState } from "react";
import { appFiles, appUrl, cliCommand, osName, release, type Os } from "@/site";

// Guesses the visitor's OS after load; the static HTML renders macOS first.
export function useOs(): [Os, boolean] {
  const [os, setOs] = useState<Os>("macos");
  const [known, setKnown] = useState(false);
  useEffect(() => {
    const ua = navigator.userAgent;
    setOs(/Windows/i.test(ua) ? "windows" : /Linux|X11|CrOS/i.test(ua) && !/Android/i.test(ua) ? "linux" : "macos");
    setKnown(true);
  }, []);
  return [os, known];
}

export function Command({ text, label }: { text: string; label: string }) {
  const [copied, setCopied] = useState(false);
  async function copy() {
    try {
      await navigator.clipboard.writeText(text);
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1600);
    } catch {
      setCopied(false);
    }
  }
  return (
    <div className="command">
      <code aria-label={label}>{text}</code>
      <button type="button" onClick={copy} className="command-copy">
        {copied ? "Copied" : "Copy"}
      </button>
    </div>
  );
}

// The landing page's primary action: the right file or command for this OS.
export function PrimaryInstall() {
  const [os] = useOs();
  const file = appFiles[os][0];
  return (
    <div className="install">
      {release.app ? (
        <a className="plate-button" href={appUrl(file.file)}>
          Download for {osName[os]}
        </a>
      ) : (
        <Link className="plate-button" href="/download/">
          Get Tunwarden
        </Link>
      )}
      <Command text={cliCommand(os)} label={`Install command for ${osName[os]}`} />
      <p className="install-note">
        {release.cli
          ? `CLI for ${osName[os]}. Free, MIT or Apache-2.0. No account, no telemetry.`
          : "No release yet: this builds the CLI from source with Rust. Free, MIT or Apache-2.0."}
      </p>
    </div>
  );
}
