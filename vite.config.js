import { defineConfig } from "vite";

// 路径 A：Tauri 直接加载远程站点，前端仅作打包兜底占位页。
// root 指向 src，使 vite 能找到 index.html；outDir 输出到项目根 dist，
// 与 tauri.conf.json 的 frontendDist "../dist"（相对 src-tauri）匹配。
export default defineConfig({
  root: "src",
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    watch: { ignored: ["**/src-tauri/**"] },
  },
  build: {
    outDir: "../dist",
    emptyOutDir: true,
  },
});
