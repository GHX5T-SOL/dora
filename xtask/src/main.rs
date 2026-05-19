use anyhow::{Context, bail};
use clap::{Parser, Subcommand};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Parser)]
#[command(name = "xtask", about = "Dora task runner")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Stage library files from build output to prefix directory
    Stage {
        /// Crate name (e.g., dora-node-api-c, dora-operator-api-c, dora-node-api-cxx, dora-operator-api-cxx)
        crate_name: String,

        /// Cargo build target directory (e.g., target/release)
        target_dir: PathBuf,

        /// Staging output directory (e.g., dora-c-libraries-x86_64-unknown-linux-gnu)
        prefix: PathBuf,
    },
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Stage {
            crate_name,
            target_dir,
            prefix,
        } => {
            stage_library(&crate_name, &target_dir, &prefix)?;
        }
    }

    Ok(())
}

fn is_cxx_crate(crate_name: &str) -> bool {
    crate_name.ends_with("-cxx")
}

fn stage_library(crate_name: &str, target_dir: &Path, prefix: &Path) -> anyhow::Result<()> {
    if is_cxx_crate(crate_name) {
        stage_cxx_crate(crate_name, target_dir, prefix)
    } else {
        stage_c_crate(crate_name, target_dir, prefix)
    }
}

fn stage_c_crate(crate_name: &str, target_dir: &Path, prefix: &Path) -> anyhow::Result<()> {
    let staging = target_dir.join(crate_name);

    if !staging.exists() {
        bail!(
            "Staging directory not found: {}\n\
             Ensure {crate_name} has been built (cargo build -p {crate_name}).",
            staging.display()
        );
    }

    let cmake_src = staging.join("lib/cmake").join(crate_name);
    let include_src = staging.join("include");

    if !cmake_src.exists() {
        bail!("CMake config directory not found: {}", cmake_src.display());
    }
    if !include_src.exists() {
        bail!("Include directory not found: {}", include_src.display());
    }

    println!("Staging {crate_name} (C crate)...");
    println!("  TARGET_DIR: {}", target_dir.display());
    println!("  STAGING:    {}", staging.display());
    println!("  PREFIX:     {}", prefix.display());

    let lib_cmake_dir = prefix.join("lib/cmake");
    let include_dir = prefix.join("include");
    fs::create_dir_all(&lib_cmake_dir)?;
    fs::create_dir_all(&include_dir)?;

    copy_dir_contents(&cmake_src, &lib_cmake_dir.join(crate_name))
        .with_context(|| format!("Failed to copy cmake config for {crate_name}"))?;

    copy_dir_contents(&include_src, &include_dir)
        .with_context(|| format!("Failed to copy headers for {crate_name}"))?;

    copy_library(crate_name, target_dir, prefix)?;

    print_staged_files(prefix, crate_name);

    Ok(())
}

fn stage_cxx_crate(crate_name: &str, target_dir: &Path, prefix: &Path) -> anyhow::Result<()> {
    // The cxxbridge artefacts live next to the cargo `target/` directory,
    // but cargo lays them out as either `target/cxxbridge/<crate>/` (host
    // builds) or `target/<triple>/cxxbridge/<crate>/` (cross builds /
    // `--target` invocations). Locate whichever exists by walking up the
    // ancestors of `target_dir` (which is `target/release`,
    // `target/<triple>/release`, or similar) and checking each one's
    // `cxxbridge/<crate>` subdirectory.
    // Prefer a candidate whose `lib/cmake/<crate>` subdir is populated —
    // otherwise we may pick up a stale `cxxbridge/<crate>` from a previous
    // build that predates the cmake-emitting build.rs.
    let cxxbridge_dir = target_dir
        .ancestors()
        .find_map(|ancestor| {
            let candidate = ancestor.join("cxxbridge").join(crate_name);
            candidate
                .join("lib/cmake")
                .join(crate_name)
                .exists()
                .then_some(candidate)
        })
        .ok_or_else(|| {
            anyhow::anyhow!(
                "Cxxbridge directory with cmake config not found for {crate_name} in any ancestor of {}.\n\
                 Ensure {crate_name} has been built (cargo build -p {crate_name}).",
                target_dir.display()
            )
        })?;

    let cmake_src = cxxbridge_dir.join("lib/cmake").join(crate_name);
    let include_src = cxxbridge_dir.join("include");
    let src_src = cxxbridge_dir.join("src");

    if !cmake_src.exists() {
        bail!("CMake config directory not found: {}", cmake_src.display());
    }
    if !include_src.exists() {
        bail!("Include directory not found: {}", include_src.display());
    }

    println!("Staging {crate_name} (C++ crate)...");
    println!("  CXXBRIDGE:  {}", cxxbridge_dir.display());
    println!("  TARGET_DIR: {}", target_dir.display());
    println!("  PREFIX:     {}", prefix.display());

    let lib_cmake_dir = prefix.join("lib/cmake");
    let include_dir = prefix.join("include");
    let src_dir = prefix.join("src");
    fs::create_dir_all(&lib_cmake_dir)?;
    fs::create_dir_all(&include_dir)?;
    fs::create_dir_all(&src_dir)?;

    copy_dir_contents(&cmake_src, &lib_cmake_dir.join(crate_name))
        .with_context(|| format!("Failed to copy cmake config for {crate_name}"))?;

    copy_dir_contents(&include_src, &include_dir)
        .with_context(|| format!("Failed to copy headers for {crate_name}"))?;

    if src_src.exists() {
        copy_dir_contents(&src_src, &src_dir)
            .with_context(|| format!("Failed to copy cxxbridge source for {crate_name}"))?;
    }

    copy_library(crate_name, target_dir, prefix)?;

    print_staged_files(prefix, crate_name);

    Ok(())
}

fn copy_dir_contents(src: &Path, dst: &Path) -> anyhow::Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let dst_path = dst.join(entry.file_name());

        if file_type.is_dir() {
            copy_dir_contents(&entry.path(), &dst_path)?;
        } else {
            fs::copy(entry.path(), &dst_path)?;
        }
    }
    Ok(())
}

fn copy_library(crate_name: &str, target_dir: &Path, prefix: &Path) -> anyhow::Result<()> {
    let lib_name = match crate_name {
        "dora-node-api-c" => {
            if cfg!(windows) {
                "dora_node_api_c.lib"
            } else {
                "libdora_node_api_c.a"
            }
        }
        "dora-operator-api-c" => {
            if cfg!(windows) {
                "dora_operator_api_c.lib"
            } else {
                "libdora_operator_api_c.a"
            }
        }
        "dora-node-api-cxx" => {
            if cfg!(windows) {
                "dora_node_api_cxx.lib"
            } else {
                "libdora_node_api_cxx.a"
            }
        }
        "dora-operator-api-cxx" => {
            if cfg!(windows) {
                "dora_operator_api_cxx.lib"
            } else {
                "libdora_operator_api_cxx.a"
            }
        }
        _ => {
            eprintln!("WARNING: Unknown crate {crate_name}, skipping library copy");
            return Ok(());
        }
    };

    // Look for the static lib at the user-supplied target_dir first
    // (the common publish-workflow case where `cargo build --target X
    // --release` puts it at `target/X/release/`). On macOS host builds,
    // cargo writes the lib to `target/<profile>/` (no triple subdir)
    // even though the cxxbridge files end up under `target/<triple>/`,
    // so also try the sibling `target/<profile>/` path. The profile is
    // the last component of target_dir.
    let lib_dst = prefix.join("lib").join(lib_name);
    let primary = target_dir.join(lib_name);
    let host_fallback = target_dir
        .file_name()
        .and_then(|profile| {
            target_dir
                .ancestors()
                .nth(2)
                .map(|root| root.join(profile).join(lib_name))
        })
        .filter(|p| p != &primary);

    let lib_src = if primary.exists() {
        Some(primary)
    } else {
        host_fallback.filter(|p| p.exists())
    };

    if let Some(lib_src) = lib_src {
        fs::copy(&lib_src, &lib_dst).with_context(|| {
            format!(
                "Failed to copy library {} -> {}",
                lib_src.display(),
                lib_dst.display()
            )
        })?;
    } else {
        eprintln!(
            "WARNING: Library file not found at {} (or sibling host profile dir)",
            target_dir.join(lib_name).display()
        );
    }

    Ok(())
}

fn print_staged_files(prefix: &Path, crate_name: &str) {
    println!("Staged files:");
    if let Ok(entries) = fs::read_dir(prefix.join("lib")) {
        for entry in entries.filter_map(|e| e.ok()) {
            println!("  lib/{}", entry.file_name().to_string_lossy());
        }
    }
    if let Ok(entries) = fs::read_dir(prefix.join("include")) {
        for entry in entries.filter_map(|e| e.ok()) {
            println!("  include/{}", entry.file_name().to_string_lossy());
        }
    }
    let cmake_dir = prefix.join("lib/cmake").join(crate_name);
    if let Ok(entries) = fs::read_dir(&cmake_dir) {
        for entry in entries.filter_map(|e| e.ok()) {
            println!(
                "  lib/cmake/{crate_name}/{}",
                entry.file_name().to_string_lossy()
            );
        }
    }
}
