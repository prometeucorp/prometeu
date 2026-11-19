import { run } from "./stdio.ts";

// Separate entrypoint allows a single CommonJS SEA bundle without import.meta or file imports.
run(process.stdin, process.stdout).catch(() => { process.exitCode = 1; });
