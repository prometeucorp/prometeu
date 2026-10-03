import { defineConfig } from "vitest/config";

export default defineConfig({
  clearScreen: false,
  // Each worktree supplies its Vite port through the environment. Keep strictPort so Tauri never
  // silently connects to another server.
  server: { port: Number(process.env.PORT ?? 1420), strictPort: true },
  build: {
    rollupOptions: { input: { app: "index.html", notification: "notification.html", designSystem: "design-system.html", companyDesignSystem: "packages/design-system/index.html" } },
    // Target the modern Tauri webview so production builds accept top-level await used during bootstrap.
    target: "esnext",
    minify: "esbuild",
  },
  // Limit Vitest to this repository; nested worktrees are not excluded automatically through .gitignore.
  test: {
    include: ["src/**/*.test.ts", "relay/src/**/*.test.ts"],
    // Relay integration tests drive a real Worker; GitHub-hosted macOS runners need more than the
    // 5 s test and 1 s poll defaults.
    testTimeout: 30_000,
    expect: { poll: { timeout: 5_000 } },
  },
});
