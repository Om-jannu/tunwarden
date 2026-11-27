import type { Metadata, Viewport } from "next";
import localFont from "next/font/local";
import "./globals.css";

const display = localFont({
  src: "../fonts/big-shoulders-latin.woff2",
  weight: "100 900",
  variable: "--font-display",
  display: "swap",
});
const plate = localFont({
  src: "../fonts/sofia-sans-condensed-latin.woff2",
  weight: "100 900",
  variable: "--font-plate",
  display: "swap",
});
const body = localFont({
  src: "../fonts/sofia-sans-latin.woff2",
  weight: "100 900",
  variable: "--font-body",
  display: "swap",
});
const mono = localFont({
  src: "../fonts/jetbrains-mono-latin.woff2",
  weight: "100 800",
  variable: "--font-mono",
  display: "swap",
});

export const metadata: Metadata = {
  metadataBase: new URL("https://tunwarden.vercel.app"),
  title: { default: "Tunwarden: keeps your tunnels running", template: "%s · Tunwarden" },
  description:
    "Tunwarden starts your SSH and cloud tunnels, checks each one really listens, reconnects what drops, and tells you when a login expires. Free and open source for macOS, Windows and Linux.",
  openGraph: { type: "website", siteName: "Tunwarden" },
};

export const viewport: Viewport = { themeColor: "#1c2420", colorScheme: "dark" };

export default function RootLayout({ children }: { children: React.ReactNode }) {
  return (
    <html lang="en" className={`${display.variable} ${plate.variable} ${body.variable} ${mono.variable}`}>
      <body>{children}</body>
    </html>
  );
}
