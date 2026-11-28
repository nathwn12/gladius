use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=.git/index");
    if let Some(reference) = git(&["symbolic-ref", "-q", "HEAD"]) {
        println!("cargo:rerun-if-changed=.git/{reference}");
    }
    println!("cargo:rerun-if-changed=.git/packed-refs");
    println!("cargo:rerun-if-changed=src");
    println!("cargo:rerun-if-changed=skills");

    let sha = git(&["rev-parse", "HEAD"]).unwrap_or_else(|| "unknown".to_string());
    let dirty = git(&["status", "--porcelain"]).is_some_and(|s| !s.is_empty());

    println!("cargo:rustc-env=GLADIUS_GIT_SHA={sha}");
    println!("cargo:rustc-env=GLADIUS_GIT_DIRTY={}", u8::from(dirty));

    generate_skills();
}

fn generate_skills() {
    use std::fs;
    use std::path::Path;
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let skills_root = Path::new(manifest_dir).join("skills");
    let out_dir = std::env::var("OUT_DIR").expect("OUT_DIR");
    let out_path = Path::new(&out_dir).join("skills_generated.rs");

    let mut skill_dirs: Vec<std::path::PathBuf> = Vec::new();
    if skills_root.exists() {
        for entry in fs::read_dir(&skills_root).expect("read skills dir") {
            let entry = entry.expect("skills entry");
            if entry.file_type().expect("file type").is_dir() {
                skill_dirs.push(entry.path());
            }
        }
    }
    skill_dirs.sort();

    fn collect_files(base: &Path, dir: &Path, out: &mut Vec<(String, std::path::PathBuf)>) {
        for entry in fs::read_dir(dir).expect("read dir") {
            let entry = entry.expect("entry");
            let path = entry.path();
            if entry.file_type().expect("ft").is_dir() {
                collect_files(base, &path, out);
            } else if entry.file_type().expect("ft").is_file() {
                let rel = path
                    .strip_prefix(base)
                    .expect("strip")
                    .to_string_lossy()
                    .replace('\\', "/");
                out.push((rel, path));
            }
        }
    }

    let manifest_norm = manifest_dir.replace('\\', "/");
    let mut code = String::new();
    code.push_str("&[\n");
    for dir in &skill_dirs {
        let name = dir.file_name().expect("name").to_string_lossy().to_string();
        let mut files: Vec<(String, std::path::PathBuf)> = Vec::new();
        collect_files(dir, dir, &mut files);
        files.sort_by(|a, b| a.0.cmp(&b.0));
        code.push_str(&format!("    EmbeddedSkill {{ name: {name:?}, files: &[\n"));
        for (rel, full) in &files {
            let full_norm = full.to_string_lossy().replace('\\', "/");
            let rel_to_manifest = full_norm
                .strip_prefix(&format!("{manifest_norm}/"))
                .expect("prefix");
            code.push_str(&format!(
                "        EmbeddedFile {{ rel_path: {rel:?}, bytes: include_bytes!(concat!(env!(\"CARGO_MANIFEST_DIR\"), \"/{rel_to_manifest}\")) }},\n",
            ));
        }
        code.push_str("    ] },\n");
    }
    code.push_str("]\n");
    fs::write(out_path, code).expect("write skills_generated.rs");
}

/// Run `git` in the package root; `None` when git is absent or the command
/// failed, so a git-less build still succeeds with `unknown`.
fn git(args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .args(args)
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
}
