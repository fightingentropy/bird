use std::env;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Deserialize;
use sha2::{Digest, Sha256};

const DISTFILE_MANIFEST_NAME: &str = "manifest.json";
const CACHE_ROOT_NAME: &str = "curl-impersonate-cache";
const SUCCESS_MARKER: &str = "build.ok";

#[derive(Debug, Deserialize)]
struct DistfileManifest {
    distfiles: Vec<DistfileEntry>,
}

#[derive(Debug, Deserialize)]
struct DistfileEntry {
    filename: String,
    url: String,
    sha256: String,
    extracted_dir: String,
}

fn main() {
    println!("cargo:rustc-check-cfg=cfg(bird_native_impersonation)");

    let manifest_dir = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let workspace_root = manifest_dir
        .parent()
        .and_then(Path::parent)
        .expect("bird-curl-impersonate-sys should live under crates/")
        .to_path_buf();
    let source_root = workspace_root.join("third_party/curl-impersonate");
    let distfiles_dir = source_root.join("distfiles");
    let manifest_path = distfiles_dir.join(DISTFILE_MANIFEST_NAME);

    register_rerun_dir(&source_root);
    println!(
        "cargo:rerun-if-changed={}",
        manifest_dir.join("build.rs").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        manifest_dir.join("src/lib.rs").display()
    );
    for name in [
        "PATH",
        "CC",
        "CXX",
        "CFLAGS",
        "CXXFLAGS",
        "SDKROOT",
        "MACOSX_DEPLOYMENT_TARGET",
        "CARGO_TARGET_DIR",
    ] {
        println!("cargo:rerun-if-env-changed={name}");
    }

    if !should_build_native_impersonation() {
        return;
    }

    println!("cargo:rustc-cfg=bird_native_impersonation");

    let configure = source_root.join("configure");
    if !configure.is_file() {
        panic!(
            "missing vendored curl-impersonate source at {}",
            source_root.display()
        );
    }
    if !manifest_path.is_file() {
        panic!(
            "missing vendored distfile manifest at {}",
            manifest_path.display()
        );
    }

    let manifest = load_manifest(&manifest_path);
    for distfile in &manifest.distfiles {
        println!(
            "cargo:rerun-if-changed={}",
            distfiles_dir.join(&distfile.filename).display()
        );
    }

    let gmake = require_tool("gmake");
    for tool in [
        "cmake",
        "ninja",
        "pkg-config",
        "go",
        "patch",
        "tar",
        "unzip",
    ] {
        require_tool(tool);
    }

    let compiler = cc::Build::new()
        .try_get_compiler()
        .expect("failed to detect a C compiler for native macOS impersonation");
    let target = env::var("TARGET").unwrap();
    let cache_dir = target_root(&workspace_root)
        .join(CACHE_ROOT_NAME)
        .join(build_fingerprint(
            &target,
            &manifest_path,
            &source_root.join("patches"),
            &manifest_dir.join("build.rs"),
            compiler.path(),
        ))
        .join(&target);
    let work_dir = cache_dir.join("work");
    let install_root = cache_dir.join("install");
    let link_dir = cache_dir.join("link");
    let success_marker = cache_dir.join(SUCCESS_MARKER);

    if !cache_is_ready(&install_root, &success_marker) {
        rebuild_cache(
            &cache_dir,
            &source_root,
            &distfiles_dir,
            &manifest,
            &configure,
            &gmake,
            &work_dir,
            &install_root,
            &success_marker,
        );
    }

    let config_bin = install_root.join("bin/curl-impersonate-config");
    let curl_archive = install_root.join("lib/libcurl-impersonate.a");
    if !config_bin.is_file() || !curl_archive.is_file() {
        panic!(
            "native macOS impersonation cache at {} is incomplete",
            cache_dir.display()
        );
    }

    create_libcurl_alias(&link_dir, &curl_archive);
    let static_libs = run_output_command(
        Command::new(&config_bin).arg("--static-libs"),
        "query vendored curl-impersonate link flags",
    );

    println!("cargo:root={}", install_root.display());
    println!("cargo:include={}", install_root.join("include").display());
    println!("cargo:static=1");
    println!("cargo:rustc-link-search=native={}", link_dir.display());
    emit_link_flags(&static_libs);
}

fn should_build_native_impersonation() -> bool {
    let target = env::var("TARGET").unwrap();
    let host = env::var("HOST").unwrap();
    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    target_os == "macos" && target == host
}

fn target_root(workspace_root: &Path) -> PathBuf {
    env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| workspace_root.join("target"))
}

fn load_manifest(path: &Path) -> DistfileManifest {
    let contents = fs::read_to_string(path).unwrap_or_else(|error| {
        panic!(
            "failed to read distfile manifest {}: {error}",
            path.display()
        )
    });
    serde_json::from_str(&contents).unwrap_or_else(|error| {
        panic!(
            "failed to parse distfile manifest {}: {error}",
            path.display()
        )
    })
}

fn build_fingerprint(
    target: &str,
    manifest_path: &Path,
    patches_dir: &Path,
    build_rs_path: &Path,
    compiler_path: &Path,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(target.as_bytes());
    hasher.update(read_bytes(manifest_path));
    hash_dir_contents(&mut hasher, patches_dir);
    hasher.update(read_bytes(build_rs_path));
    hasher.update(compiler_path.to_string_lossy().as_bytes());
    if let Ok(output) = Command::new(compiler_path).arg("--version").output() {
        hasher.update(&output.stdout);
        hasher.update(&output.stderr);
    }
    format!("{:x}", hasher.finalize())
}

fn read_bytes(path: &Path) -> Vec<u8> {
    fs::read(path).unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()))
}

fn hash_dir_contents(hasher: &mut Sha256, dir: &Path) {
    let mut files = collect_files(dir);
    files.sort();
    for file in files {
        hasher.update(file.to_string_lossy().as_bytes());
        hasher.update(read_bytes(&file));
    }
}

fn collect_files(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let entries = fs::read_dir(dir)
        .unwrap_or_else(|error| panic!("failed to read directory {}: {error}", dir.display()));
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            files.extend(collect_files(&path));
        } else {
            files.push(path);
        }
    }
    files
}

fn cache_is_ready(install_root: &Path, success_marker: &Path) -> bool {
    success_marker.is_file()
        && install_root.join("bin/curl-impersonate-config").is_file()
        && install_root.join("lib/libcurl-impersonate.a").is_file()
}

fn rebuild_cache(
    cache_dir: &Path,
    source_root: &Path,
    distfiles_dir: &Path,
    manifest: &DistfileManifest,
    configure: &Path,
    gmake: &Path,
    work_dir: &Path,
    install_root: &Path,
    success_marker: &Path,
) {
    if cache_dir.exists() {
        fs::remove_dir_all(cache_dir).unwrap_or_else(|error| {
            panic!(
                "failed to clear native impersonation cache {}: {error}",
                cache_dir.display()
            )
        });
    }
    fs::create_dir_all(work_dir).unwrap();
    fs::create_dir_all(install_root).unwrap();

    seed_distfiles(distfiles_dir, manifest, work_dir);

    run_command(
        Command::new(configure)
            .current_dir(work_dir)
            .arg(format!("--prefix={}", install_root.display())),
        "configure vendored curl-impersonate",
    );
    run_command(
        Command::new(gmake).current_dir(work_dir).arg("build"),
        "build vendored curl-impersonate",
    );
    run_command(
        Command::new(gmake).current_dir(work_dir).arg("install"),
        "install vendored curl-impersonate",
    );

    for distfile in &manifest.distfiles {
        let extracted_dir = work_dir.join(&distfile.extracted_dir);
        if !extracted_dir.exists() {
            panic!(
                "vendored distfile {} did not unpack to expected directory {}",
                distfile.filename,
                extracted_dir.display()
            );
        }
    }

    if !source_root.join("patches").is_dir() {
        panic!(
            "missing vendored patch set at {}",
            source_root.join("patches").display()
        );
    }

    fs::write(success_marker, b"ok\n").unwrap_or_else(|error| {
        panic!(
            "failed to write native impersonation cache marker {}: {error}",
            success_marker.display()
        )
    });
}

fn seed_distfiles(distfiles_dir: &Path, manifest: &DistfileManifest, work_dir: &Path) {
    for distfile in &manifest.distfiles {
        let source_path = distfiles_dir.join(&distfile.filename);
        if !source_path.is_file() {
            panic!(
                "missing vendored distfile {} (expected at {}, upstream {})",
                distfile.filename,
                source_path.display(),
                distfile.url
            );
        }
        let digest = sha256_file(&source_path);
        if digest != distfile.sha256 {
            panic!(
                "SHA-256 mismatch for {}: expected {}, got {}",
                source_path.display(),
                distfile.sha256,
                digest
            );
        }

        let target_path = work_dir.join(&distfile.filename);
        if target_path.exists() {
            fs::remove_file(&target_path).unwrap();
        }
        symlink_or_copy(&source_path, &target_path);
    }
}

fn sha256_file(path: &Path) -> String {
    let mut file = fs::File::open(path)
        .unwrap_or_else(|error| panic!("failed to open {}: {error}", path.display()));
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 8192];
    loop {
        let count = file
            .read(&mut buffer)
            .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()));
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    format!("{:x}", hasher.finalize())
}

fn create_libcurl_alias(link_dir: &Path, curl_archive: &Path) {
    fs::create_dir_all(link_dir).unwrap();
    let alias = link_dir.join("libcurl.a");
    if alias.exists() {
        fs::remove_file(&alias).unwrap();
    }
    symlink_or_copy(curl_archive, &alias);
}

#[cfg(unix)]
fn symlink_or_copy(source: &Path, target: &Path) {
    use std::os::unix::fs::symlink;

    symlink(source, target).unwrap_or_else(|error| {
        panic!(
            "failed to link {} -> {}: {error}",
            target.display(),
            source.display()
        )
    });
}

#[cfg(not(unix))]
fn symlink_or_copy(source: &Path, target: &Path) {
    fs::copy(source, target).unwrap_or_else(|error| {
        panic!(
            "failed to copy {} -> {}: {error}",
            source.display(),
            target.display()
        )
    });
}

fn register_rerun_dir(path: &Path) {
    println!("cargo:rerun-if-changed={}", path.display());
    let entries = match fs::read_dir(path) {
        Ok(entries) => entries,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let child = entry.path();
        if child.is_dir() {
            register_rerun_dir(&child);
        } else {
            println!("cargo:rerun-if-changed={}", child.display());
        }
    }
}

fn require_tool(name: &str) -> PathBuf {
    find_in_path(name).unwrap_or_else(|| {
        panic!(
            "missing required build tool `{name}` on PATH; install the macOS native impersonation prerequisites first"
        )
    })
}

fn find_in_path(name: &str) -> Option<PathBuf> {
    let path = env::var_os("PATH")?;
    env::split_paths(&path)
        .map(|dir| dir.join(name))
        .find(|candidate| candidate.is_file())
}

fn emit_link_flags(static_libs: &str) {
    let mut tokens = static_libs.split_whitespace().peekable();
    while let Some(token) = tokens.next() {
        if token == "-framework" {
            let framework = tokens
                .next()
                .unwrap_or_else(|| panic!("missing framework name after -framework"));
            println!("cargo:rustc-link-lib=framework={framework}");
            continue;
        }
        if token == "-pthread" {
            continue;
        }
        if let Some(path) = token.strip_prefix("-L") {
            println!("cargo:rustc-link-search=native={path}");
            continue;
        }
        if let Some(lib) = token.strip_prefix("-l") {
            println!("cargo:rustc-link-lib={lib}");
            continue;
        }
        if token.ends_with(".a") {
            let library = Path::new(token);
            let stem = library
                .file_stem()
                .and_then(|value| value.to_str())
                .unwrap_or_else(|| panic!("static library path `{token}` is missing a file stem"));
            let lib_name = stem.strip_prefix("lib").unwrap_or(stem);
            if lib_name == "curl-impersonate" {
                continue;
            }
            let parent = library
                .parent()
                .unwrap_or_else(|| panic!("static library path `{token}` is missing a parent"));
            println!("cargo:rustc-link-search=native={}", parent.display());
            println!("cargo:rustc-link-lib=static={lib_name}");
        }
    }
}

fn run_command(command: &mut Command, description: &str) {
    let status = command
        .status()
        .unwrap_or_else(|error| panic!("failed to {description}: {error}"));
    if !status.success() {
        panic!("failed to {description}: exited with status {status}");
    }
}

fn run_output_command(command: &mut Command, description: &str) -> String {
    let output = command
        .output()
        .unwrap_or_else(|error| panic!("failed to {description}: {error}"));
    if !output.status.success() {
        panic!(
            "failed to {description}: exited with status {}",
            output.status
        );
    }
    String::from_utf8(output.stdout)
        .unwrap_or_else(|error| panic!("failed to decode {description} output: {error}"))
}
