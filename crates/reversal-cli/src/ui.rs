//! Local web UI: a small HTTP server on 127.0.0.1 serving one page and a JSON API.
//! Requests from other sites are refused, see [`request_allowed`].
//!
//! GET  /api/files                          raw files in the folder
//! GET  /api/load?file=NAME                 decode (cached), auto and saved settings, warning
//! POST /api/estimate {file, hints}         re-run the analysis with manual areas
//! POST /api/preview  {file, settings, full} JPEG of the positive; `full` ignores the crop.
//!                                          Header X-Clipped: share of pixels the DNG can't hold
//! POST /api/settings {file, settings}      remember a frame's settings (its sidecar)
//! POST /api/export   {files}               save DNGs in the background, each with its own settings
//! GET  /api/export/status                  progress of that
//! POST /api/export/cancel                  frames not started yet don't start
//! POST /api/roll     {files}               measure one film base across frames and use it on
//!                                          each; runs like an export, with the same progress

use std::collections::HashMap;
use std::io::Cursor;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use reversal_core::dng;
use reversal_core::image::{NRect, Rgb32};
use reversal_core::invert::{self, Adjust, Hints, Settings};
use reversal_core::render;
use serde::Deserialize;
use serde_json::json;
use tiny_http::{Header, Method, Request, Response, Server};

use crate::batch::{Batch, Done};
use crate::folder::{saved_settings, Folder};

const INDEX_HTML: &str = include_str!("../ui/index.html");

type Resp = Response<Cursor<Vec<u8>>>;
type Handled = Result<Resp, (u16, String)>;

struct Loaded {
    /// Downsampled scan: used for previews and (re-)estimation.
    small: Rgb32,
    auto: Settings,
}

struct State {
    port: u16,
    folder: Arc<Folder>,
    cache: HashMap<String, Loaded>,
    batch: Batch,
}

#[derive(Deserialize)]
struct Job {
    file: String,
    settings: Settings,
    #[serde(default)]
    full: bool,
}

#[derive(Deserialize)]
struct SaveSettings {
    file: String,
    settings: Settings,
}

#[derive(Deserialize)]
struct ExportJob {
    files: Vec<String>,
}

#[derive(Deserialize)]
struct EstimateJob {
    file: String,
    hints: Hints,
}

/// Serve `dir` on `port`, or on a free port if it's 0. With `exit_with_stdin` the process ends
/// when stdin closes, so it can't outlive the app that started it.
pub fn serve(
    dir: PathBuf,
    port: u16,
    exit_with_stdin: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let dir = dir.canonicalize()?;
    let server = Server::http(("127.0.0.1", port))
        .map_err(|e| format!("cannot listen on port {port}: {e}"))?;
    let port = server
        .server_addr()
        .to_ip()
        .map_or(port, |address| address.port());
    if exit_with_stdin {
        std::thread::spawn(|| {
            let _ = std::io::copy(&mut std::io::stdin(), &mut std::io::sink());
            std::process::exit(0);
        });
    }
    println!(
        "Reversal UI: http://127.0.0.1:{port}  (folder: {})",
        dir.display()
    );
    let mut state = State {
        port,
        folder: Arc::new(Folder::new(dir)),
        cache: HashMap::new(),
        batch: Batch::default(),
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
            (Method::Post, "/api/settings") => {
                let job: SaveSettings = read_body(req)?;
                self.save_settings(&job)
            }
            (Method::Post, "/api/export") => {
                let job: ExportJob = read_body(req)?;
                self.export(job.files)
            }
            (Method::Post, "/api/roll") => {
                let job: ExportJob = read_body(req)?;
                self.measure_roll(job.files)
            }
            (Method::Get, "/api/export/status") => Ok(json_resp(200, &self.batch.status())),
            (Method::Post, "/api/export/cancel") => {
                self.batch.cancel();
                Ok(json_resp(200, &self.batch.status()))
            }
            _ => Err((404, format!("not found: {path}"))),
        }
    }

    fn files(&self) -> Handled {
        let files: Vec<_> = self
            .folder
            .raw_files()
            .into_iter()
            .map(|name| {
                let state = self.folder.frame_state(&name);
                json!({
                    "name": name,
                    "saved": state.saved,
                    "edited": state.edited,
                    "manualSetup": state.manual_setup,
                })
            })
            .collect();
        let folder = self.folder.dir.display().to_string();
        Ok(json_resp(200, &json!({ "folder": folder, "files": files })))
    }

    fn ensure_loaded(&mut self, file: &str) -> Result<&Loaded, (u16, String)> {
        if !self.cache.contains_key(file) {
            let path = self.folder.resolve(file)?;
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
        let sidecar = std::fs::read(self.folder.sidecar(file));
        let loaded = self.ensure_loaded(file)?;
        let auto = loaded.auto.clone();
        let (current, warning) = match sidecar {
            Ok(bytes) => saved_settings(&bytes, &auto, &loaded.small),
            Err(_) => (auto.clone(), None),
        };
        Ok(json_resp(
            200,
            &json!({
                "auto": auto,
                "settings": current,
                "warning": warning,
                "defaultAreaPercent": invert::DEFAULT_AREA_PERCENT,
            }),
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
        let (profile, _) = reversal_core::profiles(&job.settings.adjust);
        let jpeg = render::preview_jpeg(&img, &profile, 88);
        Ok(Response::from_data(jpeg)
            .with_header(header("Content-Type", "image/jpeg"))
            .with_header(header("Cache-Control", "no-store"))
            .with_header(header("X-Clipped", &format!("{clipped:.6}"))))
    }

    fn save_settings(&self, job: &SaveSettings) -> Handled {
        self.folder.resolve(&job.file)?;
        self.folder
            .save_settings(&job.file, &job.settings)
            .map_err(|e| (500, e.to_string()))?;
        let state = self.folder.frame_state(&job.file);
        Ok(json_resp(
            200,
            &json!({ "saved": state.saved, "edited": state.edited, "manualSetup": state.manual_setup }),
        ))
    }

    fn export(&self, files: Vec<String>) -> Handled {
        for file in &files {
            self.folder.resolve(file)?;
        }
        let folder = self.folder.clone();
        self.batch
            .start(files, export_workers(), move |file| {
                let t = Instant::now();
                let (out, bytes, warning) = folder.export_frame(file)?;
                println!(
                    "exported {} ({:.1} MB, {:.1} s)",
                    out.display(),
                    bytes as f64 / 1e6,
                    t.elapsed().as_secs_f64()
                );
                let path = out.display().to_string();
                Ok(Done { path, warning })
            })
            .map_err(|e| (409, e))?;
        Ok(json_resp(200, &self.batch.status()))
    }
}

impl State {
    fn measure_roll(&self, files: Vec<String>) -> Handled {
        if files.len() < 2 {
            return Err((400, "a roll needs at least two frames".to_string()));
        }
        for file in &files {
            self.folder.resolve(file)?;
        }
        let measured = Arc::new(Mutex::new(Vec::with_capacity(files.len())));
        let total = files.len();
        let (folder, collected) = (self.folder.clone(), measured.clone());
        let apply_folder = self.folder.clone();
        self.batch
            .start_then(
                "roll",
                files,
                1,
                move |file| {
                    let frame = folder.measure_for_roll(file)?;
                    collected
                        .lock()
                        .expect("no panics while locked")
                        .push(frame);
                    Ok(Done {
                        path: String::new(),
                        warning: None,
                    })
                },
                move || {
                    let frames =
                        std::mem::take(&mut *measured.lock().expect("no panics while locked"));
                    if frames.len() < total {
                        return Some(
                            "Not every frame could be read, so nothing changed.".to_string(),
                        );
                    }
                    apply_folder.apply_roll_base(&frames).err()
                },
            )
            .map_err(|e| (409, e))?;
        Ok(json_resp(200, &self.batch.status()))
    }
}

/// Frames saved at once. Each also uses all cores while converting, and holds a few hundred MB.
fn export_workers() -> usize {
    let cores = std::thread::available_parallelism().map_or(1, |n| n.get());
    (cores / 3).clamp(1, 4)
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

    #[test]
    fn decodes_query_values() {
        assert_eq!(
            query_param("/api/load?file=_DSC5384.NEF", "file").as_deref(),
            Some("_DSC5384.NEF")
        );
        assert_eq!(percent_decode("a%20b+c%2"), "a b c%2");
    }
}
