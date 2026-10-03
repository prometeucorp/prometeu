//! Conservative source checks complement the core's independent build on Linux and Windows.

use std::path::Path;

#[test]
fn core_dependencies_do_not_reach_desktop_or_native_adapters() {
    let manifest: toml::Value = toml::from_str(include_str!("../Cargo.toml")).unwrap();
    let dependencies = manifest["dependencies"].as_table().unwrap();
    for (name, dependency) in dependencies {
        assert!(
            ["serde", "serde_json", "uuid"].contains(&name.as_str()),
            "review the core boundary before adding {name}"
        );
        assert!(
            dependency.get("path").is_none(),
            "core must not depend on an adapter"
        );
        assert!(
            dependency.get("package").is_none(),
            "dependency aliases require boundary review"
        );
    }
    assert!(
        manifest.get("target").is_none(),
        "platform dependencies belong to adapters"
    );
    assert!(manifest.get("build-dependencies").is_none());
}

fn check_sources(directory: &Path) {
    for entry in std::fs::read_dir(directory).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            check_sources(&path);
            continue;
        }
        if path.extension().is_none_or(|extension| extension != "rs") {
            continue;
        }
        let source = std::fs::read_to_string(&path).unwrap();
        let compact: String = source.chars().filter(|c| !c.is_whitespace()).collect();
        for forbidden in [
            "tauri::",
            "AppHandle",
            "AppState",
            "std::fs",
            "std::process",
            "std::net",
            "std::os",
            "std::{fs",
            "std::{process",
            "std::{net",
            "std::{os",
            "target_os",
            "target_family",
            "target_env",
            "cfg(unix)",
            "cfg(windows)",
            "cfg!(unix)",
            "cfg!(windows)",
        ] {
            assert!(
                !compact.contains(forbidden),
                "{} reaches {forbidden}",
                path.display()
            );
        }
    }
}

#[test]
fn core_sources_keep_native_effects_and_platform_selection_at_the_edge() {
    check_sources(&Path::new(env!("CARGO_MANIFEST_DIR")).join("src"));
}
