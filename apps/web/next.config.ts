import type { NextConfig } from "next";

// A static site: `next build` writes plain HTML to `out/`, which Vercel serves
// as is. No server, no image optimisation, no analytics.
const config: NextConfig = {
  output: "export",
  trailingSlash: true,
  images: { unoptimized: true },
  poweredByHeader: false,
};

export default config;
