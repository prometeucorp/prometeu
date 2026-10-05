//! The IPC command map, Rust handlers, and browser mock must expose the same names.
//! Argument and result shapes are checked by TypeScript for callers and mock handlers.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf()
}

fn read(rel: &str) -> String {
    std::fs::read_to_string(repo().join(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

fn declared() -> BTreeSet<String> {
    read("src/ipc.ts")
        .lines()
        .filter_map(|line| line.strip_prefix("  ")?.split_once(": { args:"))
        .map(|(name, _)| name.to_string())
        .collect()
}

fn registered() -> BTreeSet<String> {
    let text = read("src-tauri/src/main.rs");
    let start = text
        .find("generate_handler![")
        .expect("missing generate_handler!");
    let block = &text[start..text[start..].find(']').unwrap() + start];
    block
        .lines()
        .filter(|line| line.contains("::"))
        .filter_map(|line| line.trim().trim_end_matches(',').rsplit("::").next())
        .map(str::to_string)
        .collect()
}

fn mocked() -> BTreeSet<String> {
    let text = read("src/mock.ts");
    let (_, handlers) = text
        .split_once("const mockCommands: IpcHandlers = {")
        .unwrap();
    let (handlers, _) = handlers.split_once("\n};").unwrap();
    handlers
        .lines()
        .filter_map(|line| line.strip_prefix("  ")?.split_once('('))
        .map(|(name, args)| (name.strip_prefix("async ").unwrap_or(name), args))
        .filter(|(name, _)| name.chars().all(|c| c.is_ascii_lowercase() || c == '_'))
        .map(|(name, _)| name.to_string())
        .collect()
}

#[test]
fn typed_commands_match_rust_handlers() {
    let commands = declared();
    assert!(commands.len() > 100, "command scanner missed the contract");
    assert_eq!(commands, registered());
}

#[test]
fn typed_commands_match_browser_handlers() {
    assert_eq!(declared(), mocked());
}
