import { fileURLToPath } from "node:url";
import { defineConfig, loadEnv } from "vite";

export default defineConfig(() => {
  // Vite resolves its client environment after evaluating this config.
  // Seed only public settings so IDE development mode also loads .env.profile.
  const directory = fileURLToPath(new URL(".", import.meta.url));
  const profile = loadEnv("profile", directory, "VITE_");
  for (const name of ["VITE_API_BASE_URL", "VITE_USER_REGISTRY_ADDRESS"]) {
    if (profile[name] !== undefined) process.env[name] = profile[name];
  }
  return {};
});
