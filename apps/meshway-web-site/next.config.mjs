import { createMDX } from "fumadocs-mdx/next"

const withMDX = createMDX()

/** @type {import('next').NextConfig} */
const config = {
  // A static site: landing, docs and the API reference, no server of its own.
  output: "export",
  reactStrictMode: true,
  transpilePackages: ["@meshway/ui", "@meshway/control-client"],
  images: { unoptimized: true },
}

export default withMDX(config)
