#![cfg(unix)]
#[path = "support/search_mcp.rs"]
mod support;

#[test]
fn bundled_search_scopes_worktrees_and_refreshes_shared_sessions() {
    support::worktree_search_contract(env!("CARGO_BIN_EXE_Prometeu"));
}
