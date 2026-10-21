//! Build script: compiles `res/nodi.rc` (icon, manifest, version info) with
//! `windres`, because the GNU toolchain ships no resource compiler of its own.
//!
//! Two environment quirks are handled here:
//!
//! * the `windres` on this machine (binutils 2.28, MinGW32) can only emit
//!   `pe-i386` objects, while cargo links `x86_64` COFF — [`retarget_object`]
//!   rewrites the machine field and relocation types to the target
//!   architecture. Resource objects contain data plus section-relative fixups
//!   only, so this translation is exact;
//! * when no `windres` can be found the build still succeeds with a warning and
//!   the app falls back to a runtime DPI call (see `REPORT.md`).

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const RES_OBJ: &str = "nodi_res.o";

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=res/nodi.rc");
    println!("cargo:rerun-if-changed=res/nodi.manifest");
    println!("cargo:rerun-if-changed=res/nodi.ico");

    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR is always set by cargo"));
    let obj = out_dir.join(RES_OBJ);

    match compile_with_windres(&obj) {
        Ok(()) => {
            retarget_object(&obj);
            println!("cargo:rustc-link-arg-bins={}", obj.display());
        }
        Err(LookupError::NotFound) => {
            println!(
                "cargo:warning=windres not found: building without embedded manifest, icon and \
                 version info. The app falls back to a runtime DPI-awareness call; Common \
                 Controls v6 styling will be unavailable."
            );
        }
        Err(LookupError::Failed(message)) => panic!("windres failed: {message}"),
    }
}

enum LookupError {
    NotFound,
    Failed(String),
}

fn compile_with_windres(obj: &Path) -> Result<(), LookupError> {
    for candidate in candidates() {
        match Command::new(&candidate).arg("--version").output() {
            Ok(out) if out.status.success() => {}
            _ => continue,
        }

        // `windres` resolves `nodi.ico` / `nodi.manifest` relative to the
        // resource script, so run it from `res/`.
        let output = Command::new(&candidate)
            .current_dir("res")
            .arg("-i")
            .arg("nodi.rc")
            .arg("-o")
            .arg(obj)
            .args(["-O", "coff"])
            .output()
            .map_err(|err| LookupError::Failed(format!("{candidate}: {err}")))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let stdout = String::from_utf8_lossy(&output.stdout);
            return Err(LookupError::Failed(format!(
                "{candidate} exited with {}: {stdout}{stderr}",
                output.status
            )));
        }
        if !obj.is_file() {
            return Err(LookupError::Failed(format!(
                "{candidate} reported success but {} is missing",
                obj.display()
            )));
        }
        return Ok(());
    }

    Err(LookupError::NotFound)
}

fn candidates() -> Vec<String> {
    let mut candidates = Vec::new();
    if let Ok(path) = env::var("WINDRES") {
        candidates.push(path);
    }
    candidates.push("windres".to_string());
    candidates.push("x86_64-w64-mingw32-windres".to_string());

    // Some rustup toolchains ship `windres` inside the self-contained MinGW;
    // this one does not, but asking is cheap and keeps the script portable.
    if let Some(sysroot) = rustc_sysroot() {
        let target = env::var("TARGET").unwrap_or_default();
        let bundled = Path::new(&sysroot)
            .join("lib")
            .join("rustlib")
            .join(target)
            .join("bin")
            .join("self-contained")
            .join("windres.exe");
        if let Some(path) = bundled.to_str() {
            candidates.push(path.to_string());
        }
    }

    candidates
}

fn rustc_sysroot() -> Option<String> {
    let rustc = env::var("RUSTC").unwrap_or_else(|_| "rustc".to_string());
    let output = Command::new(rustc)
        .args(["--print", "sysroot"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8(output.stdout).ok()?;
    let trimmed = text.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

const IMAGE_FILE_MACHINE_I386: u16 = 0x014c;
const IMAGE_FILE_MACHINE_AMD64: u16 = 0x8664;
const IMAGE_FILE_MACHINE_ARM64: u16 = 0xaa64;

/// IMAGE_SCN_CNT_CODE | IMAGE_SCN_MEM_EXECUTE: never expected in a resource object.
const SECTION_CODE_OR_EXECUTE: u32 = 0x0000_0020 | 0x2000_0000;

const COFF_HEADER_SIZE: usize = 20;
const SECTION_HEADER_SIZE: usize = 40;
const RELOCATION_SIZE: usize = 10;

/// Rewrites a `pe-i386` COFF object into the machine type cargo builds for.
///
/// A resource object is pure data: the `.rsrc` tree points into its own section
/// through `DIR32NB`/`SECREL` fixups, and no machine code or
/// architecture-specific relocation (branch, pdata, unwind) is involved.
/// Translating the machine field plus the relocation types is therefore exact.
fn retarget_object(obj: &Path) {
    let target = match env::var("CARGO_CFG_TARGET_ARCH").as_deref() {
        Ok("x86_64") => IMAGE_FILE_MACHINE_AMD64,
        Ok("aarch64") => IMAGE_FILE_MACHINE_ARM64,
        Ok("x86") => IMAGE_FILE_MACHINE_I386,
        Ok(other) => panic!("windres resources are not supported on target arch {other}"),
        Err(_) => panic!("CARGO_CFG_TARGET_ARCH is always set by cargo"),
    };

    let mut bytes =
        fs::read(obj).unwrap_or_else(|err| panic!("cannot read {}: {err}", obj.display()));
    let source = read_u16(&bytes, 0);
    if source == target {
        return;
    }
    if source != IMAGE_FILE_MACHINE_I386 {
        panic!(
            "{}: unexpected resource object machine 0x{source:04x}",
            obj.display()
        );
    }

    let sections = read_u16(&bytes, 2) as usize;
    for index in 0..sections {
        let header = COFF_HEADER_SIZE + index * SECTION_HEADER_SIZE;
        let characteristics = read_u32(&bytes, header + 36);
        if characteristics & SECTION_CODE_OR_EXECUTE != 0 {
            panic!(
                "{}: resource object contains code (section {index}), refusing to retarget",
                obj.display()
            );
        }

        let count = read_u16(&bytes, header + 32) as usize;
        let table = read_u32(&bytes, header + 24) as usize;
        for reloc in 0..count {
            let entry = table + reloc * RELOCATION_SIZE;
            let kind = read_u16(&bytes, entry + 8);
            let translated: u16 = match kind {
                // IMAGE_REL_I386_DIR32 -> IMAGE_REL_AMD64_ADDR32
                0x0006 => 0x0002,
                // IMAGE_REL_I386_DIR32NB -> IMAGE_REL_AMD64_ADDR32NB
                0x0007 => 0x0003,
                // IMAGE_REL_I386_SECTION -> IMAGE_REL_AMD64_SECTION
                0x000a => 0x000a,
                // IMAGE_REL_I386_SECREL -> IMAGE_REL_AMD64_SECREL
                0x000b => 0x000b,
                other => panic!(
                    "{}: cannot translate i386 relocation type 0x{other:04x}",
                    obj.display()
                ),
            };
            bytes[entry + 8..entry + 10].copy_from_slice(&translated.to_le_bytes());
        }
    }

    bytes[0..2].copy_from_slice(&target.to_le_bytes());
    fs::write(obj, &bytes).unwrap_or_else(|err| panic!("cannot rewrite {}: {err}", obj.display()));
}

fn read_u16(bytes: &[u8], offset: usize) -> u16 {
    let slice = bytes
        .get(offset..offset + 2)
        .unwrap_or_else(|| panic!("truncated COFF object at offset {offset}"));
    u16::from_le_bytes([slice[0], slice[1]])
}

fn read_u32(bytes: &[u8], offset: usize) -> u32 {
    let slice = bytes
        .get(offset..offset + 4)
        .unwrap_or_else(|| panic!("truncated COFF object at offset {offset}"));
    u32::from_le_bytes([slice[0], slice[1], slice[2], slice[3]])
}
