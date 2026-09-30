import type { NextConfig } from "next"

const nextConfig: NextConfig = {
  transpilePackages: ["@meshway/control-client"],
  // Lets a review server build into its own directory next to a running
  // production build. Unset means the default `.next`.
  distDir: process.env.MESHWAY_WEB_DIST_DIR || ".next",
}

export default nextConfig
