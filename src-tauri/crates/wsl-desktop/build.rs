use sha2::{Digest, Sha256};
fn main() {
    println!("cargo:rerun-if-env-changed=PROMETEU_WSL_RUNTIME");
    let out = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    let declaration = match std::env::var_os("PROMETEU_WSL_RUNTIME") {
        Some(path) => {
            let path = std::path::PathBuf::from(path)
                .canonicalize()
                .expect("Linux runtime package must exist");
            println!("cargo:rerun-if-changed={}", path.display());
            let bytes = std::fs::read(&path).expect("read Linux runtime package");
            assert!(
                bytes.len() >= 20
                    && bytes.len() <= 128 * 1024 * 1024
                    && &bytes[..4] == b"\x7fELF"
                    && bytes[4] == 2
                    && bytes[5] == 1
                    && bytes[18..20] == [62, 0],
                "runtime package must be an x86_64 Linux ELF executable"
            );
            let digest = format!("{:x}", Sha256::digest(&bytes));
            let packaged = out.join("prometeu-runtime-linux");
            std::fs::write(&packaged, bytes).expect("copy runtime package");
            format!("pub const RUNTIME_PACKAGE: Option<(&str, &[u8])> = Some(({digest:?}, include_bytes!({:?})));", packaged.to_str().unwrap())
        }
        None => "pub const RUNTIME_PACKAGE: Option<(&str, &[u8])> = None;".into(),
    };
    std::fs::write(out.join("runtime_package.rs"), declaration).unwrap();
    #[cfg(feature = "desktop")]
    tauri_build::build();
}
