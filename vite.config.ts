import { cpSync, createReadStream, existsSync, statSync } from "fs";
import { join, resolve } from "path";
import { defineConfig, type Plugin } from "vite";

// @ts-expect-error process is a nodejs global
const host = process.env.TAURI_DEV_HOST;

const RESOURCES_SRC = resolve(__dirname, "src-tauri/resources");
const RESOURCES_URL = "/resources";

/** Sert `src-tauri/resources/` en dev et copie dans `dist/resources/` au build (installable Tauri). */
function tauriResourcesPlugin(): Plugin {
  return {
    name: "tauri-resources",
    configureServer(server) {
      server.middlewares.use(RESOURCES_URL, (req, res, next) => {
        const raw = req.url?.split("?")[0] ?? "/";
        const rel = decodeURIComponent(raw);
        if (!rel || rel.includes("..")) {
          res.statusCode = 400;
          res.end();
          return;
        }
        const filePath = join(RESOURCES_SRC, rel);
        if (!filePath.startsWith(RESOURCES_SRC) || !existsSync(filePath) || statSync(filePath).isDirectory()) {
          next();
          return;
        }
        const ext = filePath.slice(filePath.lastIndexOf(".")).toLowerCase();
        const types: Record<string, string> = {
          ".json": "application/json",
          ".png": "image/png",
          ".models": "application/json",
        };
        res.setHeader("Content-Type", types[ext] ?? "application/octet-stream");
        createReadStream(filePath).pipe(res);
      });
    },
    closeBundle() {
      cpSync(RESOURCES_SRC, resolve(__dirname, "dist/resources"), { recursive: true });
    },
  };
}

// https://vite.dev/config/
export default defineConfig(async () => ({
  plugins: [tauriResourcesPlugin()],

  // Vite options tailored for Tauri development and only applied in `tauri dev` or `tauri build`
  //
  // 1. prevent Vite from obscuring rust errors
  clearScreen: false,
  // 2. tauri expects a fixed port, fail if that port is not available
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: 1421,
        }
      : undefined,
    watch: {
      // 3. tell Vite to ignore watching `src-tauri`
      ignored: ["**/src-tauri/**"],
    },
  },
  build: {
    rollupOptions: {
      input: {
        main: resolve(__dirname, "index.html"),
        models: resolve(__dirname, "models.html"),
      },
    },
  },
}));
