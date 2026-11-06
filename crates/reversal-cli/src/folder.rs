//! A folder of scans: which frames it holds, where their results and settings go.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use reversal_core::dng::Compression;
use reversal_core::image::Rgb32;
use reversal_core::invert::{self, Adjust, Hints, Settings, MODEL_VERSION};
use serde::Serialize;

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
}

/// What one frame's film base reads, for the user to review before a roll uses it.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct RollReading {
    pub file: String,
    pub base_rgb: [f32; 3],
    /// Where it came from: "picked" by hand, "detected" on clear film, or "unsure" if no clear
    /// film was found.
    pub source: &'static str,
}

pub struct FrameState {
    /// A DNG of this frame exists.
    pub saved: bool,
    /// The settings differ from that DNG's, or from the automatic ones if there is none.
    pub edited: bool,
    /// The film base or measured area were set by hand.
    pub manual_setup: bool,
}

/// The parts of a sidecar that are the user's, readable from any model version.
#[derive(serde::Deserialize, Default)]
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
    let warning = "These settings came from an older version. Colors were measured again, \
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

/// Clipped film is worth a message when saving. A holder or bare light isn't, as saving many
/// frames would report nearly all of them: they cost the picture nothing, and Setup shows them.
fn film_clipping_warning(small: &Rgb32, settings: &Settings) -> Option<String> {
    let clipping = invert::measure_clipping(small, settings);
    if clipping.harms_film() {
        clipping.warning(settings.adjust.accept_clipping)
    } else {
        None
    }
}

fn settings_json(settings: &Settings) -> Vec<u8> {
    serde_json::to_vec_pretty(settings).expect("settings serialize")
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

    /// The settings a frame's DNG was written with.
    fn in_dng(&self, file: &str) -> PathBuf {
        self.sidecar(file).with_extension("in-dng.json")
    }

    pub fn frame_state(&self, file: &str) -> FrameState {
        let choices = |path: &Path| -> SavedChoices {
            std::fs::read(path)
                .ok()
                .and_then(|bytes| serde_json::from_slice(&bytes).ok())
                .unwrap_or_default()
        };
        let saved = self.output_path(file).is_file();
        let now = choices(&self.sidecar(file));
        let before = if saved {
            choices(&self.in_dng(file))
        } else {
            SavedChoices::default()
        };
        FrameState {
            saved,
            edited: now.hints != before.hints || now.adjust != before.adjust,
            manual_setup: now.hints.base.is_some()
                || now.hints.analysis.is_some()
                || now.hints.area_percent.is_some(),
        }
    }

    pub fn save_settings(&self, file: &str, settings: &Settings) -> std::io::Result<()> {
        let _changing = self.sidecars.lock().unwrap_or_else(|e| e.into_inner());
        self.write_settings(file, settings)
    }

    /// Keep a frame's adjustments and whatever is measured on disk: the page's measurements
    /// may be older, when a roll is rewriting them.
    pub fn save_adjustments(&self, file: &str, settings: &Settings) -> std::io::Result<()> {
        let _changing = self.sidecars.lock().unwrap_or_else(|e| e.into_inner());
        let on_disk = std::fs::read(self.sidecar(file))
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Settings>(&bytes).ok());
        match on_disk {
            Some(mut saved) => {
                saved.adjust = settings.adjust.clone();
                self.write_settings(file, &saved)
            }
            None => self.write_settings(file, settings),
        }
    }

    fn write_settings(&self, file: &str, settings: &Settings) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.out_dir)?;
        reversal_core::write_atomic(&self.sidecar(file), &settings_json(settings))
    }

    fn roll_image(&self, file: &str) -> Result<Rgb32, String> {
        let input = self.resolve(file).map_err(|(_, message)| message)?;
        let scan = reversal_core::decode::decode(&input).map_err(|e| e.to_string())?;
        Ok(invert::analysis_image(&scan))
    }

    /// The film base a frame gives on its own: the one picked on it, or the one detected.
    /// Any earlier roll base is left out. Changes nothing.
    pub fn read_roll_base(&self, file: &str) -> Result<RollReading, String> {
        let small = self.roll_image(file)?;
        let sidecar = std::fs::read(self.sidecar(file)).ok();
        let (settings, _) = settings_from(sidecar.as_deref(), &small);
        let reading = |base_rgb, source| RollReading {
            file: file.to_string(),
            base_rgb,
            source,
        };
        if settings.hints.base.is_some() {
            let hints = Hints {
                base_rgb: None,
                ..settings.hints
            };
            return Ok(reading(
                invert::estimate_on(&small, &hints).base_rgb,
                "picked",
            ));
        }
        let detected = invert::detect_base(&small);
        let source = if detected.found { "detected" } else { "unsure" };
        Ok(reading(detected.rgb, source))
    }

    /// What applying a roll base to a frame needs, read before anything is written.
    pub fn prepare_roll_frame(&self, file: &str) -> Result<RollFrame, String> {
        Ok(RollFrame {
            file: file.to_string(),
            small: self.roll_image(file)?,
        })
    }

    /// Give frames one film base, keeping everything else the user chose, also what changed
    /// while preparing. A base picked by hand on a frame stays.
    pub fn apply_roll_base(&self, frames: &[RollFrame], base_rgb: [f32; 3]) -> Result<(), String> {
        for frame in frames {
            let _changing = self.sidecars.lock().unwrap_or_else(|e| e.into_inner());
            let sidecar = std::fs::read(self.sidecar(&frame.file)).ok();
            let (mut settings, _) = settings_from(sidecar.as_deref(), &frame.small);
            settings.hints.base_rgb = Some(base_rgb);
            settings.estimate = invert::estimate_on(&frame.small, &settings.hints);
            self.write_settings(&frame.file, &settings)
                .map_err(|e| format!("{}: {e}", frame.file))?;
        }
        Ok(())
    }

    /// Write a frame's DNG with its remembered settings, or the automatic ones if it has none.
    /// Returns the path, the size in bytes, and anything the user should know.
    pub fn export_frame(&self, file: &str) -> Result<(PathBuf, u64, Option<String>), String> {
        let input = self.resolve(file).map_err(|(_, message)| message)?;
        let out = self.output_path(file);
        pipeline::replaceable(&out)?;
        let scan = reversal_core::decode::decode(&input).map_err(|e| e.to_string())?;
        let small = invert::analysis_image(&scan);
        let sidecar_path = self.sidecar(file);
        let used = std::fs::read(&sidecar_path).ok();
        let (settings, warning) = settings_from(used.as_deref(), &small);
        let warning = warning.or_else(|| film_clipping_warning(&small, &settings));
        std::fs::create_dir_all(&self.out_dir).map_err(|e| e.to_string())?;
        let bytes = pipeline::export(&input, &scan, &settings, Compression::LosslessJpeg, &out)
            .map_err(|e| e.to_string())?;
        // What's in the DNG, not what's in the sidecar: that may have changed while this ran.
        reversal_core::write_atomic(&self.in_dng(file), &settings_json(&settings))
            .map_err(|e| e.to_string())?;
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
        settings.adjust.exposure = 20.0;
        folder.save_settings("a.NEF", &settings).unwrap();
        let state = folder.frame_state("a.NEF");
        assert!(!state.saved && state.edited && !state.manual_setup);

        std::fs::write(folder.output_path("a.NEF"), b"dng").unwrap();
        std::fs::write(folder.in_dng("a.NEF"), settings_json(&settings)).unwrap();
        let state = folder.frame_state("a.NEF");
        assert!(state.saved && !state.edited);

        settings.hints.base = Some(settings.estimate.base_area);
        folder.save_settings("a.NEF", &settings).unwrap();
        let state = folder.frame_state("a.NEF");
        assert!(state.saved && state.edited && state.manual_setup);
    }

    #[test]
    fn going_back_to_where_a_frame_was_clears_edited() {
        let temp = TempFolder::new("back");
        let folder = &temp.0;
        let auto = self::settings();
        let mut changed = auto.clone();
        changed.adjust.whites = -30.0;

        // Never saved: the automatic settings are where it started.
        folder.save_settings("a.NEF", &changed).unwrap();
        assert!(folder.frame_state("a.NEF").edited);
        folder.save_settings("a.NEF", &auto).unwrap();
        assert!(!folder.frame_state("a.NEF").edited);

        // Saved: the DNG's settings are, even when they aren't the automatic ones.
        folder.save_settings("a.NEF", &changed).unwrap();
        std::fs::write(folder.output_path("a.NEF"), b"dng").unwrap();
        std::fs::write(folder.in_dng("a.NEF"), settings_json(&changed)).unwrap();
        folder.save_settings("a.NEF", &auto).unwrap();
        assert!(folder.frame_state("a.NEF").edited);
        folder.save_settings("a.NEF", &changed).unwrap();
        assert!(!folder.frame_state("a.NEF").edited);
    }

    #[test]
    fn saving_adjustments_keeps_what_a_roll_measured() {
        let temp = TempFolder::new("adjustments");
        let folder = &temp.0;
        let before_roll = self::settings();
        let mut rolled = before_roll.clone();
        rolled.hints.base_rgb = Some([0.3, 0.4, 0.25]);
        rolled.estimate.base_rgb = [0.3, 0.4, 0.25];
        folder.save_settings("a.NEF", &rolled).unwrap();

        let mut edited = before_roll;
        edited.adjust.exposure = 42.0;
        folder.save_adjustments("a.NEF", &edited).unwrap();

        let bytes = std::fs::read(folder.sidecar("a.NEF")).unwrap();
        let saved: Settings = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(saved.adjust.exposure, 42.0);
        assert_eq!(saved.hints, rolled.hints);
        assert_eq!(saved.estimate.base_rgb, rolled.estimate.base_rgb);
    }

    #[test]
    fn saving_adjustments_of_a_frame_without_a_sidecar_writes_them_all() {
        let temp = TempFolder::new("first-adjustments");
        let folder = &temp.0;
        let mut edited = self::settings();
        edited.adjust.exposure = 42.0;
        folder.save_adjustments("a.NEF", &edited).unwrap();
        let bytes = std::fs::read(folder.sidecar("a.NEF")).unwrap();
        let saved: Settings = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(saved, edited);
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
