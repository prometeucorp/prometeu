import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import ts from "typescript";

// A literal inferred from JSON preserves discriminants and nulls that JSON imports widen.
// Assignment through a variable permits additive backend fields, as the wire contract requires.
const fixture = readFileSync("fixtures/backend-contract.json", "utf8").trim();
const filename = resolve("src/backend-contract-check.ts");
const source = `
import type { IpcResult } from "./ipc";
import type { ProviderId } from "./types";
import type { AgentCapabilities } from "./agents";
import type { AnyConversationEventV1 } from "./conversation";
type ReadonlyWire<T> = { readonly [K in keyof T]: ReadonlyWire<T[K]> };
const payload = ${fixture} as const;
export const board: ReadonlyWire<IpcResult<"load_board">> = payload.board;
export const snapshot: ReadonlyWire<IpcResult<"chat_snapshot">> = payload.snapshot;
export const models: ReadonlyWire<IpcResult<"agent_models">> = payload.models;
export const providers: ReadonlyWire<ProviderId[]> = payload.providers;
export const capabilities: ReadonlyWire<Record<ProviderId, AgentCapabilities>> = payload.capabilities;
export const events: ReadonlyWire<Record<string, AnyConversationEventV1[]>> = payload.events;
`;
const config = ts.readConfigFile("tsconfig.json", ts.sys.readFile);
if (config.error) throw new Error(ts.flattenDiagnosticMessageText(config.error.messageText, "\n"));
const parsed = ts.parseJsonConfigFileContent(config.config, ts.sys, process.cwd());
const host = ts.createCompilerHost(parsed.options);
const original = host.getSourceFile.bind(host);
host.getSourceFile = (path, ...args) => resolve(path) === filename
  ? ts.createSourceFile(path, source, parsed.options.target, true)
  : original(path, ...args);
const declarations = parsed.fileNames.filter(path => path.endsWith(".d.ts"));
const program = ts.createProgram([...declarations, filename], parsed.options, host);
const diagnostics = [...parsed.errors, ...ts.getPreEmitDiagnostics(program)];
if (diagnostics.length) {
  console.error(ts.formatDiagnosticsWithColorAndContext(diagnostics, {
    getCanonicalFileName: name => name,
    getCurrentDirectory: () => process.cwd(),
    getNewLine: () => "\n",
  }));
  process.exitCode = 1;
} else {
  console.log("Rust fixture payloads match the TypeScript IPC and conversation contracts.");
}
