import type { NextConfig } from "next";

// Tauri serves the static export; there is no Node server at runtime.
const nextConfig: NextConfig = {
  output: "export",
  images: { unoptimized: true },
  trailingSlash: true,
  agentRules: false,
};

export default nextConfig;
