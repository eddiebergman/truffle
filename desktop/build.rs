use std::fmt::Write as _;

fn files(current: &std::path::Path, result: &mut Vec<std::path::PathBuf>) {
    for item in std::fs::read_dir(current).unwrap() {
        let path = item.unwrap().path();
        if path.is_dir() {
            files(&path, result);
        } else if path.is_file() {
            result.push(path);
        }
    }
}

fn main() {
    println!(
        "cargo:rustc-env=TRUFFLE_BUILD_TIME={}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
    );
    let root = std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap())
        .join("../static")
        .canonicalize()
        .unwrap();
    let mut paths = Vec::new();
    files(&root, &mut paths);
    paths.sort();
    let mut source = String::from("pub fn asset(path:&str)->Option<&'static [u8]>{match path{\n");
    let config_names = paths
        .iter()
        .map(|p| {
            p.strip_prefix(&root)
                .unwrap()
                .to_string_lossy()
                .into_owned()
        })
        .filter(|p| p.starts_with("config/"))
        .collect::<Vec<_>>();
    for path in paths {
        println!("cargo:rerun-if-changed={}", path.display());
        writeln!(
            source,
            "{:?}=>Some(include_bytes!({:?})),",
            path.strip_prefix(&root).unwrap().to_string_lossy(),
            path.to_string_lossy()
        )
        .unwrap();
    }
    source.push_str("_=>None}}\n");
    writeln!(source, "pub const CONFIG_FILES:&[&str]=&{config_names:?};").unwrap();
    std::fs::write(
        std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("assets.rs"),
        source,
    )
    .unwrap();
    println!("cargo:rerun-if-changed={}", root.display());
    tauri_build::build();
}
