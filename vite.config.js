import { defineConfig } from "vite";

// 路径 A：Tauri 直接加载远程站点，前端仅作打包兜底占位页。
// root 指向 src，使 vite 能找到 index.html；outDir 输出到项目根 dist，
// 与 tauri.conf.json 的 frontendDist "../dist"（相对 src-tauri）匹配。
export default defineConfig({
  root: "src",
  clearScreen: false,
  server: {
    // 别用 Tauri 模板默认的 1420：Windows 上 Hyper-V / WSL 会动态保留 1348–1447
    // 这类端口段，落在里面时 vite 绑定 ::1:1420 会报 EACCES: permission denied
    // （不是端口被占用，netstat 查不到任何监听）。5173 不在保留段内。
    // 排查：netsh interface ipv4 show excludedportrange protocol=tcp
    port: 5173,
    strictPort: true,
    watch: { ignored: ["**/src-tauri/**"] },
  },
  build: {
    outDir: "../dist",
    emptyOutDir: true,
    rollupOptions: {
      // 悬浮窗口栏是独立的本地页面，需要和 index.html 一起打包。
      input: {
        index: "src/index.html",
        dock: "src/dock.html",
      },
    },
  },
});
