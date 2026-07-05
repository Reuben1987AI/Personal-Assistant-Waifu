// Hard block: prevent `npm install`/`pnpm install`/`yarn install` on the host.
// Bun skips lifecycle scripts by default; the vite.config.ts guard and
// AGENTS.md rules handle the bun case.

const fs = require("node:fs");

const IN_DOCKER = fs.existsSync("/.dockerenv");

if (!IN_DOCKER) {
  console.error(
    "HOST BLOCK: package installs must run inside the Docker container.\n" +
    "Use `make dev-shell` to get an interactive shell, then run your command\n" +
    "inside the container. See AGENTS.md for details."
  );
  process.exit(1);
}
