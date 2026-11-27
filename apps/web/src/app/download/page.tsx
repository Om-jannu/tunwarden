import type { Metadata } from "next";
import { Footer, Nameboard } from "@/components/Chrome";
import { DownloadDesk } from "@/components/DownloadDesk";

export const metadata: Metadata = {
  title: "Download",
  description: "Download Tunwarden for macOS, Windows or Linux: the desktop app, or the CLI with one install command.",
};

export default function Download() {
  return (
    <>
      <Nameboard current="download" />
      <main className="page">
        <h1 className="page-title">Download</h1>
        <p className="section-lede">
          The desktop app includes the CLI. If you only want the CLI and daemon, one command installs it. Either way,
          Tunwarden runs your own tools, so install ssh, gcloud, kubectl or whatever your tunnels use.
        </p>
        <DownloadDesk />
      </main>
      <Footer />
    </>
  );
}
