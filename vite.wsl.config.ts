import { defineConfig } from "vite";

export default defineConfig({
  clearScreen: false,
  plugins: [{ name: "windows-composition", transformIndexHtml: { order: "pre", handler(html, context) {
    if (context.path === "/index.html" || context.path === "/") return html.replace('/src/main.ts', '/src/windows/main.ts');
    return html;
  } } }],
  // `npm run app` passes each worktree's port; the default matches tauri.conf.json's devUrl.
  // Windows locks the DLLs cargo is writing, and watching them crashes Vite with EBUSY; Rust
  // changes reload through tauri dev, not Vite.
  server: { port: Number(process.env.PORT ?? 1421), strictPort: true, watch: { ignored: ["**/src-tauri/**"] } },
  // Embed small image assets so WebView2 renders nested SVG images without another protocol request.
  build: { assetsInlineLimit: 8192, outDir: "dist-wsl", target: "esnext" },
});
