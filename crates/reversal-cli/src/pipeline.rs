//! Shared file handling for the CLI and the web UI.

use std::path::{Path, PathBuf};

use reversal_core::dng::Compression;
use reversal_core::image::Rgb32;
use reversal_core::invert::Settings;

pub const SOFTWARE: &str = concat!("Reversal ", env!("CARGO_PKG_VERSION"));

pub fn output_name(input: &Path) -> String {
    let stem = input.file_stem().unwrap_or_default().to_string_lossy();
    format!("{stem}-positive.dng")
}

/// A path in comparable form, also for files that don't exist yet.
fn resolved(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| {
        let parent = match path.parent() {
            Some(p) if !p.as_os_str().is_empty() => p,
            _ => Path::new("."),
        };
        match (parent.canonicalize(), path.file_name()) {
            (Ok(dir), Some(name)) => dir.join(name),
            _ => path.to_path_buf(),
        }
    })
}

/// Where each input goes. Fails before any work if an output would replace a source
/// file or another output of this run.
pub fn output_paths(inputs: &[PathBuf], output: Option<&Path>) -> Result<Vec<PathBuf>, String> {
    let sources: Vec<PathBuf> = inputs.iter().map(|i| resolved(i)).collect();
    let mut seen: Vec<PathBuf> = Vec::new();
    let mut outs = Vec::new();
    for input in inputs {
        let out = match output {
            Some(o) if inputs.len() == 1 && !o.is_dir() => o.to_path_buf(),
            Some(dir) => dir.join(output_name(input)),
            None => input.with_file_name(output_name(input)),
        };
        let target = resolved(&out);
        if sources.contains(&target) {
            return Err(format!(
                "{} is a source file; refusing to overwrite it",
                out.display()
            ));
        }
        if seen.contains(&target) {
            return Err(format!(
                "more than one input would be written to {}",
                out.display()
            ));
        }
        seen.push(target);
        outs.push(out);
    }
    Ok(outs)
}

/// Convert a decoded scan and write the DNG; returns the file size in bytes.
pub fn export(
    input: &Path,
    scan: &Rgb32,
    settings: &Settings,
    compression: Compression,
    out: &Path,
) -> std::io::Result<u64> {
    let source = input.file_name().unwrap_or_default().to_string_lossy();
    reversal_core::write_positive(out, &source, scan, settings, compression, SOFTWARE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_outputs_sit_next_to_inputs() {
        let outs = output_paths(&[PathBuf::from("a/x.NEF")], None).unwrap();
        assert_eq!(outs, [PathBuf::from("a/x-positive.dng")]);
    }

    #[test]
    fn refuses_to_overwrite_a_source() {
        let dir = std::env::temp_dir().join(format!("reversal-src-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let src = dir.join("x.dng");
        std::fs::write(&src, b"raw").unwrap();
        // The same file, spelled differently.
        let other = dir.join(".").join("x.dng");
        assert!(output_paths(std::slice::from_ref(&src), Some(&other)).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn refuses_colliding_outputs() {
        let inputs = [PathBuf::from("roll1/x.NEF"), PathBuf::from("roll2/x.NEF")];
        assert!(output_paths(&inputs, Some(Path::new("out"))).is_err());
        let inputs = [PathBuf::from("roll1/x.NEF"), PathBuf::from("roll1/y.NEF")];
        assert!(output_paths(&inputs, Some(Path::new("out"))).is_ok());
    }
}
