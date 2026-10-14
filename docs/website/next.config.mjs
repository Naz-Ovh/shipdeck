import { createMDX } from 'fumadocs-mdx/next';

const withMDX = createMDX();

/** @type {import('next').NextConfig} */
const config = {
  output: 'export',
  reactStrictMode: true,
  // keep `next dev` from writing AGENTS.md / CLAUDE.md into this folder
  agentRules: false,
};

export default withMDX(config);
