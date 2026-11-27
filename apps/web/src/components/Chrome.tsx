import Link from "next/link";
import { icon } from "@/art/pixels";
import { repo } from "@/site";
import { Pixels } from "./Pixels";

export function Nameboard({ current }: { current?: "download" | "docs" }) {
  return (
    <header className="nameboard">
      <Link href="/" className="nameboard-name">
        <Pixels art={icon} scale={1} className="nameboard-icon" />
        Tunwarden
      </Link>
      <nav aria-label="Main">
        <Link href="/download/" aria-current={current === "download" ? "page" : undefined}>
          Download
        </Link>
        <Link href="/docs/" aria-current={current === "docs" ? "page" : undefined}>
          Docs
        </Link>
        <a href={repo}>GitHub</a>
      </nav>
    </header>
  );
}

export function Footer() {
  return (
    <footer className="footer">
      <p>
        Tunwarden is free and open source under MIT or Apache-2.0, at your option. Source, issues and releases are on{" "}
        <a href={repo}>GitHub</a>.
      </p>
      <p>This site sets no cookies and runs no analytics. The app makes no network calls of its own.</p>
    </footer>
  );
}
