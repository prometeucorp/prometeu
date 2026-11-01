//! Desktop catalogs and effects compose the shared native startup implementation.
pub use prometeu_tools::NativeTools;
pub fn native() -> NativeTools {
    NativeTools {
        packages: std::sync::Arc::new(crate::plugins::native_packages()),
        sources: std::sync::Arc::new(crate::mcp::Sources),
        files: std::sync::Arc::new(crate::mcp::PrivateFiles),
        search: prometeu_tools::search::Search {
            executable: std::env::current_exe().expect("running application executable"),
            cache: crate::paths::root().join("search"),
        },
    }
}
