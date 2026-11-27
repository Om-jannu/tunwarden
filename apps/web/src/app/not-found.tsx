import Link from "next/link";
import { warden } from "@/art/pixels";
import { Footer, Nameboard } from "@/components/Chrome";
import { Pixels } from "@/components/Pixels";

export default function NotFound() {
  return (
    <>
      <Nameboard />
      <main className="page lost">
        <Pixels art={warden} scale={6} className="lost-warden" label="The Tunwarden warden" />
        <h1 className="page-title">No track here.</h1>
        <p className="section-lede">This page does not exist. The warden checked every tunnel.</p>
        <p className="close-links">
          <Link href="/">Back to the box</Link>
          <Link href="/docs/">Docs</Link>
        </p>
      </main>
      <Footer />
    </>
  );
}
