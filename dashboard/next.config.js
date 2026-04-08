/** @type {import('next').NextConfig} */
const config = {
  reactStrictMode: true,
  // In Next.js 15, this moved out of experimental
  serverExternalPackages: ['better-sqlite3'],
};

module.exports = config;
