//! A folder of scans: which frames it holds, where their results and settings go.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::SystemTime;

use reversal_core::dng::Compression;
use reversal_core::image::Rgb32;
use reversal_core::invert::{self, Adjust, Hints, Settings, MODEL_VERSION};

use crate::pipeline;

/// What `convert` writes next to the scans; these are results, not frames to invert.
const POSITIVE_SUFFIX: &str = "-positive.dng";
const RAW_EXTENSIONS: &[&str] = &[
    "nef", "nrw", "cr2", "cr3", "arw", "raf", "rw2", "orf", "pef", "srw", "dng",
];

pub struct Folder {
    pub dir: PathBuf,
    out_dir: PathBuf,
    /// Held while changing a sidecar: the page and background work both do.
    sidecars: Mutex<()>,
}

/// Where a frame stands, as the frame list shows it.
pub struct RollFrame {
    file: String,
    small: Rgb32,
    base_rgb: [f32; 3],
}

pub struct FrameState {
    /// A DNG of this frame exists.
    pub saved: bool,
    /// The settings changed since that DNG, or it was never saved.
    pub edited: bool,
    /// The film base or measured area were set by hand.
    pub manual_setup: bool,
}

/// The parts of a sidecar that are the user's, readable from any model version.
#[derive(serde::Deserialize)]
struct SavedChoices {
    #[serde(default)]
    model: u32,
    #[serde(default)]
    hints: Hints,
    #[serde(default)]
    adjust: Adjust,
}

/// Settings from a sidecar, for a frame with analysis image `small` and automatic settings
/// `auto`. Anything the user should know about the result comes back as a warning.
pub fn saved_settings(
    sidecar: &[u8],
    auto: &Settings,
    small: &Rgb32,
) -> (Settings, Option<String>) {
    let unreadable = |e: serde_json::Error| {
        let warning = format!("Saved settings couldn't be read ({e}). Saving will replace them.");
        (auto.clone(), Some(warning))
    };
    let choices: SavedChoices = match serde_json::from_slice(sidecar) {
        Ok(c) => c,
        Err(e) => return unreadable(e),
    };
    if choices.model == MODEL_VERSION {
        return match serde_json::from_slice(sidecar) {
            Ok(saved) => (saved, None),
            Err(e) => unreadable(e),
        };
    }
    // An older model's measurements mean nothing now: measure again, keep the user's choices.
    let mut settings = Settings::new(invert::estimate_on(small, &choices.hints), choices.adjust);
    settings.hints = choices.hints;
    let warning = "These settings came from an older version. Colours were measured again, \
                   and your adjustments may need another look.";
    (settings, Some(warning.to_string()))
}

/// A frame's settings: from its sidecar, or the automatic ones if it has none.
fn settings_from(sidecar: Option<&[u8]>, small: &Rgb32) -> (Settings, Option<String>) {
    let auto = Settings::new(
        invert::estimate_on(small, &Hints::default()),
        Adjust::default(),
    );
    match sidecar {
        Some(bytes) => saved_settings(bytes, &auto, small),
        None => (auto, None),
    }
}

impl Folder {
    pub fn new(dir: PathBuf) -> Self {
        Folder {
            out_dir: dir.join("positives"),
            dir,
            sidecars: Mutex::new(()),
        }
    }

    pub fn raw_files(&self) -> Vec<String> {
        let mut files: Vec<String> = std::fs::read_dir(&self.dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| e.path().is_file())
            .filter_map(|e| e.file_name().into_string().ok())
            .filter(|n| {
                let ext = Path::new(n)
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("");
                RAW_EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str())
                    && !n.to_ascii_lowercase().ends_with(POSITIVE_SUFFIX)
            })
            .collect();
        files.sort();
        files
    }

    /// Only names listed in the folder are accepted (no path traversal).
    pub fn resolve(&self, file: &str) -> Result<PathBuf, (u16, String)> {
        if self.raw_files().iter().any(|f| f == file) {
            Ok(self.dir.join(file))
        } else {
            Err((404, format!("unknown file: {file}")))
        }
    }

    /// Keyed on the full file name: `a.NEF` and `a.DNG` are different frames.
    pub fn sidecar(&self, file: &str) -> PathBuf {
        let name = Path::new(file)
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        self.out_dir.join(format!("{name}.reversal.json"))
    }

    /// The usual `<stem>-positive.dng`, or the full file name if another scan shares the stem.
    pub fn output_path(&self, file: &str) -> PathBuf {
        let stem = |n: &str| {
            let stem = Path::new(n).file_stem().unwrap_or_default();
            stem.to_string_lossy().to_ascii_lowercase()
        };
        let sharing = self
            .raw_files()
            .iter()
            .filter(|f| stem(f) == stem(file))
            .count();
        self.out_dir.join(if sharing > 1 {
            format!("{file}-positive.dng")
        } else {
            pipeline::output_name(Path::new(file))
        })
    }

    pub fn frame_state(&self, file: &str) -> FrameState {
        let modified =
            |path: &Path| -> Option<SystemTime> { path.metadata().ok()?.modified().ok() };
        let sidecar_path = self.sidecar(file);
        let settings_at = modified(&sidecar_path);
        let dng_at = modified(&self.output_path(file));
        let hints = settings_at
            .and_then(|_| std::fs::read(&sidecar_path).ok())
            .and_then(|bytes| serde_json::from_slice::<SavedChoices>(&bytes).ok())
            .map(|choices| choices.hints)
            .unwrap_or_default();
        FrameState {
            saved: dng_at.is_some(),
            edited: settings_at.is_some_and(|s| dng_at.is_none_or(|d| s > d)),
            manual_setup: hints.base.is_some()
                || hints.analysis.is_some()
                || hints.area_percent.is_some(),
        }
    }

    pub fn save_settings(&self, file: &str, settings: &Settings) -> std::io::Result<()> {
        let _changing = self.sidecars.lock().unwrap_or_else(|e| e.into_inner());
        self.write_settings(file, settings)
    }

    fn write_settings(&self, file: &str, settings: &Settings) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.out_dir)?;
        let bytes = serde_json::to_vec_pretty(settings).expect("settings serialise");
        reversal_core::write_atomic(&self.sidecar(file), &bytes)
    }

    /// What a frame adds to a roll measurement: its analysis image and the film base its own
    /// hints give, leaving out any earlier roll base.
    pub fn measure_for_roll(&self, file: &str) -> Result<RollFrame, String> {
        let input = self.resolve(file).map_err(|(_, message)| message)?;
        let scan = reversal_core::decode::decode(&input).map_err(|e| e.to_string())?;
        let small = invert::analysis_image(&scan);
        let sidecar = std::fs::read(self.sidecar(file)).ok();
        let (settings, _) = settings_from(sidecar.as_deref(), &small);
        let hints = Hints {
            base_rgb: None,
            ..settings.hints
        };
        let base_rgb = invert::estimate_on(&small, &hints).base_rgb;
        Ok(RollFrame {
            file: file.to_string(),
            small,
            base_rgb,
        })
    }

    /// Give frames the median of their bases, keeping everything else the user chose, also
    /// what changed while measuring. A base picked by hand on a frame stays.
    pub fn apply_roll_base(&self, frames: &[RollFrame]) -> Result<(), String> {
        let bases: Vec<[f32; 3]> = frames.iter().map(|f| f.base_rgb).collect();
        let base_rgb = invert::roll_base(&bases);
        for frame in frames {
            let _changing = self.sidecars.lock().unwrap_or_else(|e| e.into_inner());
            let sidecar = std::fs::read(self.sidecar(&frame.file)).ok();
            let (mut settings, _) = settings_from(sidecar.as_deref(), &frame.small);
            let followed_area = settings.adjust.crop == Some(settings.estimate.area);
            settings.hints.base_rgb = Some(base_rgb);
            settings.estimate = invert::estimate_on(&frame.small, &settings.hints);
            if followed_area {
                settings.adjust.crop = Some(settings.estimate.area);
            }
            self.write_settings(&frame.file, &settings)
                .map_err(|e| format!("{}: {e}", frame.file))?;
        }
        Ok(())
    }

    /// Write a frame's DNG with its remembered settings, or the automatic ones if it has none.
    /// Returns the path, the size in bytes, and anything the user should know.
    pub fn export_frame(&self, file: &str) -> Result<(PathBuf, u64, Option<String>), String> {
        let input = self.resolve(file).map_err(|(_, message)| message)?;
        let scan = reversal_core::decode::decode(&input).map_err(|e| e.to_string())?;
        let small = invert::analysis_image(&scan);
        let sidecar_path = self.sidecar(file);
        let used = std::fs::read(&sidecar_path).ok();
        let (settings, warning) = settings_from(used.as_deref(), &small);
        std::fs::create_dir_all(&self.out_dir).map_err(|e| e.to_string())?;
        let out = self.output_path(file);
        let bytes = pipeline::export(&input, &scan, &settings, Compression::LosslessJpeg, &out)
            .map_err(|e| e.to_string())?;
        // Edited while this ran? Then those settings aren't in the DNG: keep them newer than it.
        let _changing = self.sidecars.lock().unwrap_or_else(|e| e.into_inner());
        if let Ok(now) = std::fs::read(&sidecar_path) {
            if used.as_ref() != Some(&now) {
                std::fs::File::options()
                    .write(true)
                    .open(&sidecar_path)
                    .and_then(|f| f.set_modified(SystemTime::now()))
                    .map_err(|e| e.to_string())?;
            }
        }
        Ok((out, bytes, warning))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A folder with one frame, deleted when the test ends.
    struct TempFolder(Folder);

    impl TempFolder {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("reversal-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(dir.join("positives")).unwrap();
            std::fs::write(dir.join("a.NEF"), b"raw").unwrap();
            TempFolder(Folder::new(dir))
        }
    }

    impl Drop for TempFolder {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0.dir);
        }
    }

    #[test]
    fn frame_state_follows_settings_and_dng() {
        let temp = TempFolder::new("state");
        let folder = &temp.0;
        let state = folder.frame_state("a.NEF");
        assert!(!state.saved && !state.edited && !state.manual_setup);

        let mut settings = self::settings();
        folder.save_settings("a.NEF", &settings).unwrap();
        let state = folder.frame_state("a.NEF");
        assert!(!state.saved && state.edited && !state.manual_setup);

        std::thread::sleep(std::time::Duration::from_millis(5));
        std::fs::write(folder.output_path("a.NEF"), b"dng").unwrap();
        let state = folder.frame_state("a.NEF");
        assert!(state.saved && !state.edited);

        std::thread::sleep(std::time::Duration::from_millis(5));
        settings.hints.base = Some(settings.estimate.base_area);
        folder.save_settings("a.NEF", &settings).unwrap();
        let state = folder.frame_state("a.NEF");
        assert!(state.saved && state.edited && state.manual_setup);
    }

    fn small_scan() -> Rgb32 {
        Rgb32 {
            width: 2,
            height: 2,
            data: vec![0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.1, 0.2, 0.3, 0.8, 0.7, 0.6],
        }
    }

    fn settings() -> Settings {
        Settings::new(
            invert::estimate_on(&small_scan(), &Hints::default()),
            Adjust::default(),
        )
    }

    #[test]
    fn saved_settings_warn_instead_of_failing_silently() {
        let (auto, small) = (settings(), small_scan());
        let mut mine = auto.clone();
        mine.adjust.exposure = 20.0;

        let (s, warning) = saved_settings(&serde_json::to_vec(&mine).unwrap(), &auto, &small);
        assert_eq!((s, warning), (mine.clone(), None));

        let (s, warning) = saved_settings(b"not json", &auto, &small);
        assert_eq!(s, auto);
        assert!(warning.is_some());
    }

    #[test]
    fn older_settings_keep_the_users_choices_and_are_measured_again() {
        let (auto, small) = (settings(), small_scan());
        let old_sidecar = json!({
            "model": 1,
            "estimate": { "d_lo": [0, 0, 0], "log_range": 3 },
            "hints": { "analysis": { "x0": 0.0, "y0": 0.0, "x1": 1.0, "y1": 0.5 } },
            "adjust": { "exposure": 20.0 },
        });
        let (s, warning) =
            saved_settings(&serde_json::to_vec(&old_sidecar).unwrap(), &auto, &small);
        assert_eq!(s.model, MODEL_VERSION);
        assert_eq!(s.adjust.exposure, 20.0);
        assert_eq!(s.hints.analysis, Some(s.estimate.area));
        assert!(warning.is_some());
    }
}
