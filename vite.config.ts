import { readFileSync } from "node:fs";
import { join } from "node:path";
import process from "node:process";
import { defineConfig, type Plugin } from "vite";
import vue from "@vitejs/plugin-vue";

const host = process.env.TAURI_DEV_HOST;

/**
 * Keeps `tauri dev` alive when a webview asks for a `<style>` block an SFC no
 * longer has.
 *
 * A WebKit webview caches the compiled modules it fetched, so after an SFC loses
 * a `<style>` block an old client can still request `?vue&type=style&index=1`.
 * `@vitejs/plugin-vue` throws `Cannot read properties of undefined (reading
 * 'scoped')` for such a request, the dev server answers 500 and the app shows an
 * error overlay. Answering with an empty style module instead keeps the reload
 * path working: the current module graph loads, the stale block simply styles
 * nothing, and the next hard reload drops the request entirely.
 */
function staleVueStyleGuard(): Plugin {
  return {
    name: "klean:stale-vue-style-guard",
    apply: "serve",
    configureServer(server) {
      // Registered before Vite's internal middlewares, so plugin-vue never sees
      // the request when the index is out of range.
      server.middlewares.use((request, response, next) => {
        const url = request.url ?? "";
        if (!url.includes(".vue") || !url.includes("type=style")) return next();

        const [path, query = ""] = url.split("?");
        const requested = /(?:^|&)index=(\d+)/.exec(query);
        if (!requested) return next();

        let source: string;
        try {
          source = readFileSync(join(server.config.root, decodeURIComponent(path)), "utf8");
        } catch {
          return next();
        }

        const blocks = source.match(/<style\b/g)?.length ?? 0;
        const index = Number(requested[1]);
        if (index < blocks) return next();

        server.config.logger.warn(
          `[klean] ignored stale style request ${path} (index ${index}, file has ${blocks}) — reload the app`,
        );
        response.statusCode = 200;
        response.setHeader("Content-Type", "application/javascript");
        response.setHeader("Cache-Control", "no-cache");
        response.end('export default "";\nif (import.meta.hot) import.meta.hot.accept();\n');
      });
    },
  };
}

// https://vite.dev/config/
export default defineConfig(() => ({
  plugins: [vue(), staleVueStyleGuard()],

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
}));
