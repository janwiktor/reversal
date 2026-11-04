//! Local web UI: a small HTTP server on 127.0.0.1 serving one page and a JSON API.
//! Requests from other sites are refused, see [`request_allowed`].
//!
//! GET  /api/files                          raw files in the folder
//! GET  /api/load?file=NAME                 decode (cached), auto and saved settings, warning
//! POST /api/estimate {file, hints}         re-run the analysis with manual areas
//! POST /api/preview  {file, settings, full} JPEG of the positive; `full` ignores the crop.
//!                                          Header X-Clipped: share of pixels the DNG can't hold
//! POST /api/export   {file, settings}      write positives/<name>-positive.dng

use std::collections::HashMap;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::time::Instant;

use reversal_core::dng::{self, Compression};
use reversal_core::image::{NRect, Rgb32};
use reversal_core::invert::{self, Adjust, Hints, Settings, MODEL_VERSION};
use reversal_core::render;
use serde::Deserialize;
use serde_json::json;
use tiny_http::{Header, Method, Request, Response, Server};

use crate::pipeline;

const INDEX_HTML: &str = include_str!("../ui/index.html");
/// What `convert` writes next to the scans; these are results, not frames to invert.
const POSITIVE_SUFFIX: &str = "-positive.dng";
const RAW_EXTENSIONS: &[&str] = &[
    "nef", "nrw", "cr2", "cr3", "arw", "raf", "rw2", "orf", "pef", "srw", "dng",
];

type Resp = Response<Cursor<Vec<u8>>>;
type Handled = Result<Resp, (u16, String)>;

struct Loaded {
    /// Downsampled scan: used for previews and (re-)estimation.
    small: Rgb32,
    auto: Settings,
}

struct State {
    port: u16,
    dir: PathBuf,
    out_dir: PathBuf,
    cache: HashMap<String, Loaded>,
}

#[derive(Deserialize)]
struct Job {
    file: String,
    settings: Settings,
    #[serde(default)]
    full: bool,
}

#[derive(Deserialize)]
struct EstimateJob {
    file: String,
    hints: Hints,
}

pub fn serve(dir: PathBuf, port: u16) -> Result<(), Box<dyn std::error::Error>> {
    let dir = dir.canonicalize()?;
    let server = Server::http(("127.0.0.1", port))
        .map_err(|e| format!("cannot listen on port {port}: {e}"))?;
    println!(
        "Reversal UI: http://127.0.0.1:{port}  (folder: {})",
        dir.display()
    );
    let mut state = State {
        port,
        out_dir: dir.join("positives"),
        dir,
        cache: HashMap::new(),
    };
    for mut req in server.incoming_requests() {
        let resp = state
            .handle(&mut req)
            .unwrap_or_else(|(code, msg)| json_resp(code, &json!({ "error": msg })));
        let _ = req.respond(resp);
    }
    Ok(())
}

fn header(name: &str, value: &str) -> Header {
    Header::from_bytes(name.as_bytes(), value.as_bytes()).expect("static header")
}

fn json_resp(code: u16, v: &serde_json::Value) -> Resp {
    Response::from_data(v.to_string().into_bytes())
        .with_status_code(code)
        .with_header(header("Content-Type", "application/json"))
}

fn query_param(url: &str, key: &str) -> Option<String> {
    let query = url.split_once('?')?.1;
    query.split('&').find_map(|kv| {
        let (k, v) = kv.split_once('=')?;
        (k == key).then(|| percent_decode(v))
    })
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok();
                match hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
                    Some(b) => {
                        out.push(b);
                        i += 3;
                        continue;
                    }
                    None => out.push(b'%'),
                }
            }
            b'+' => out.push(b' '),
            b => out.push(b),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn request_header<'a>(req: &'a Request, name: &'static str) -> Option<&'a str> {
    req.headers()
        .iter()
        .find(|h| h.field.equiv(name))
        .map(|h| h.value.as_str())
}

/// Binding to loopback doesn't stop a web page in the user's browser from reaching this
/// server. The Host check defeats DNS rebinding, the Origin check other sites, and
/// requiring JSON forces a CORS preflight, which this server never grants.
fn request_allowed(
    port: u16,
    is_post: bool,
    host: Option<&str>,
    origin: Option<&str>,
    content_type: Option<&str>,
) -> bool {
    let local = |authority: &str| {
        authority == format!("127.0.0.1:{port}") || authority == format!("localhost:{port}")
    };
    let is_json = |c: &str| {
        let media_type = c.split(';').next().unwrap_or("").trim();
        media_type.eq_ignore_ascii_case("application/json")
    };
    host.is_some_and(local)
        && origin.is_none_or(|o| o.strip_prefix("http://").is_some_and(local))
        && (!is_post || content_type.is_some_and(is_json))
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
fn saved_settings(sidecar: &[u8], auto: &Settings, small: &Rgb32) -> (Settings, Option<String>) {
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

fn read_body<T: for<'de> Deserialize<'de>>(req: &mut Request) -> Result<T, (u16, String)> {
    serde_json::from_reader(req.as_reader()).map_err(|e| (400, format!("bad request body: {e}")))
}

impl State {
    fn handle(&mut self, req: &mut Request) -> Handled {
        let allowed = request_allowed(
            self.port,
            *req.method() == Method::Post,
            request_header(req, "Host"),
            request_header(req, "Origin"),
            request_header(req, "Content-Type"),
        );
        if !allowed {
            return Err((403, "request refused".to_string()));
        }
        let url = req.url().to_string();
        let path = url.split('?').next().unwrap_or("");
        match (req.method(), path) {
            (Method::Get, "/") => Ok(Response::from_data(INDEX_HTML.as_bytes().to_vec())
                .with_header(header("Content-Type", "text/html; charset=utf-8"))),
            (Method::Get, "/api/files") => self.files(),
            (Method::Get, "/api/load") => {
                let file = query_param(&url, "file").ok_or((400, "missing file".to_string()))?;
                self.load(&file)
            }
            (Method::Post, "/api/estimate") => {
                let job: EstimateJob = read_body(req)?;
                let loaded = self.ensure_loaded(&job.file)?;
                let estimate = invert::estimate_on(&loaded.small, &job.hints);
                Ok(json_resp(200, &json!(estimate)))
            }
            (Method::Post, "/api/preview") => {
                let job: Job = read_body(req)?;
                self.preview(&job)
            }
            (Method::Post, "/api/export") => {
                let job: Job = read_body(req)?;
                self.export(&job)
            }
            _ => Err((404, format!("not found: {path}"))),
        }
    }

    fn raw_files(&self) -> Vec<String> {
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
    fn resolve(&self, file: &str) -> Result<PathBuf, (u16, String)> {
        if self.raw_files().iter().any(|f| f == file) {
            Ok(self.dir.join(file))
        } else {
            Err((404, format!("unknown file: {file}")))
        }
    }

    /// Keyed on the full file name: `a.NEF` and `a.DNG` are different frames.
    fn sidecar(&self, file: &str) -> PathBuf {
        let name = Path::new(file)
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        self.out_dir.join(format!("{name}.reversal.json"))
    }

    /// The usual `<stem>-positive.dng`, or the full file name if another scan shares the stem.
    fn output_path(&self, file: &str) -> PathBuf {
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

    fn files(&self) -> Handled {
        let files: Vec<_> = self
            .raw_files()
            .into_iter()
            .map(|name| {
                let exported = self.sidecar(&name).exists();
                json!({ "name": name, "exported": exported })
            })
            .collect();
        Ok(json_resp(
            200,
            &json!({ "folder": self.dir.display().to_string(), "files": files }),
        ))
    }

    fn ensure_loaded(&mut self, file: &str) -> Result<&Loaded, (u16, String)> {
        if !self.cache.contains_key(file) {
            let path = self.resolve(file)?;
            let t = Instant::now();
            let scan = reversal_core::decode::decode(&path).map_err(|e| (500, e.to_string()))?;
            let small = invert::analysis_image(&scan);
            let auto = Settings::new(
                invert::estimate_on(&small, &Hints::default()),
                Adjust::default(),
            );
            println!("loaded {file} in {:.1} s", t.elapsed().as_secs_f64());
            if self.cache.len() > 16 {
                self.cache.clear();
            }
            self.cache.insert(file.to_string(), Loaded { small, auto });
        }
        Ok(&self.cache[file])
    }

    fn load(&mut self, file: &str) -> Handled {
        let sidecar = std::fs::read(self.sidecar(file));
        let loaded = self.ensure_loaded(file)?;
        let auto = loaded.auto.clone();
        let (current, warning) = match sidecar {
            Ok(bytes) => saved_settings(&bytes, &auto, &loaded.small),
            Err(_) => (auto.clone(), None),
        };
        Ok(json_resp(
            200,
            &json!({ "auto": auto, "settings": current, "warning": warning }),
        ))
    }

    fn preview(&mut self, job: &Job) -> Handled {
        let loaded = self.ensure_loaded(&job.file)?;
        let full = job.full.then_some(NRect::FULL);
        let img = invert::convert_with(&loaded.small, &job.settings, full);
        // Measured on the picture: a holder left in the scan is meant to go, not to be kept.
        let picture = job
            .settings
            .adjust
            .crop
            .unwrap_or(job.settings.estimate.area);
        let clipped = dng::clipped_share(&invert::convert_with(
            &loaded.small,
            &job.settings,
            Some(picture),
        ));
        let jpeg = render::preview_jpeg(&img, job.settings.adjust.contrast, 88);
        Ok(Response::from_data(jpeg)
            .with_header(header("Content-Type", "image/jpeg"))
            .with_header(header("Cache-Control", "no-store"))
            .with_header(header("X-Clipped", &format!("{clipped:.6}"))))
    }

    fn export(&mut self, job: &Job) -> Handled {
        let input = self.resolve(&job.file)?;
        let t = Instant::now();
        let scan = reversal_core::decode::decode(&input).map_err(|e| (500, e.to_string()))?;
        std::fs::create_dir_all(&self.out_dir).map_err(|e| (500, e.to_string()))?;
        let out = self.output_path(&job.file);
        let bytes = pipeline::export(
            &input,
            &scan,
            &job.settings,
            Compression::LosslessJpeg,
            &out,
        )
        .map_err(|e| (500, e.to_string()))?;
        let sidecar = serde_json::to_vec_pretty(&job.settings).expect("settings serialise");
        reversal_core::write_atomic(&self.sidecar(&job.file), &sidecar)
            .map_err(|e| (500, e.to_string()))?;
        let secs = t.elapsed().as_secs_f64();
        println!(
            "exported {} ({:.1} MB, {secs:.1} s)",
            out.display(),
            bytes as f64 / 1e6
        );
        Ok(json_resp(
            200,
            &json!({ "path": out.display().to_string(), "bytes": bytes, "seconds": secs }),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn allowed(host: &str, origin: Option<&str>, content_type: Option<&str>) -> bool {
        request_allowed(8787, true, Some(host), origin, content_type)
    }

    #[test]
    fn only_this_page_may_call_the_server() {
        let json = Some("application/json");
        assert!(allowed("127.0.0.1:8787", None, json));
        assert!(allowed(
            "localhost:8787",
            Some("http://localhost:8787"),
            json
        ));
        assert!(allowed(
            "127.0.0.1:8787",
            None,
            Some("application/json; charset=utf-8")
        ));
        // DNS rebinding, another site, a form-style POST that skips the preflight.
        assert!(!allowed("evil.example:8787", None, json));
        assert!(!allowed(
            "127.0.0.1:8787",
            Some("http://evil.example"),
            json
        ));
        assert!(!allowed("127.0.0.1:8787", Some("null"), json));
        assert!(!allowed("127.0.0.1:8787", None, Some("text/plain")));
        assert!(!allowed("127.0.0.1:8787", None, None));
        assert!(!allowed("127.0.0.1:9999", None, json));
        // GET needs no content type, but still the right host.
        assert!(request_allowed(
            8787,
            false,
            Some("localhost:8787"),
            None,
            None
        ));
        assert!(!request_allowed(8787, false, None, None, None));
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

    #[test]
    fn decodes_query_values() {
        assert_eq!(
            query_param("/api/load?file=_DSC5384.NEF", "file").as_deref(),
            Some("_DSC5384.NEF")
        );
        assert_eq!(percent_decode("a%20b+c%2"), "a b c%2");
    }
}
