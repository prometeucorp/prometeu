import { defineConfig } from "vite";

export default defineConfig({
  clearScreen: false,
  plugins: [{ name: "windows-composition", transformIndexHtml: { order: "pre", handler(html, context) {
    if (context.path === "/index.html" || context.path === "/") return html.replace('/src/main.ts', '/src/windows/main.ts');
    return html;
  } } }],
  server: { port: 1421, strictPort: true },
  // Embed small image assets so WebView2 renders nested SVG images without another protocol request.
  build: { assetsInlineLimit: 8192, outDir: "dist-wsl", target: "esnext", rollupOptions: { input: ["index.html", "wsl.html"] } },
});
