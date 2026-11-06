//! Local web UI: a small HTTP server on 127.0.0.1 serving one page and a JSON API.
//! Requests from other sites are refused, see [`request_allowed`], and the page runs under a
//! content security policy that allows only its own files, see [`CONTENT_SECURITY_POLICY`].
//!
//! GET  /, /app.js, /style.css             the page
//! GET  /api/version                        version and the commit it was built from
//! GET  /api/license                        the license text
//! GET  /api/files                          raw files in the folder
//! GET  /api/load?file=NAME                 decode (cached), auto and saved settings, warning
//! POST /api/estimate {file, hints}         re-run the analysis with manual areas
//! POST /api/holder   {file, base_rgb}       where the analysis ignores the film holder
//! POST /api/preview  {file, settings, full} JPEG of the positive; `full` ignores the crop.
//!                                          Header X-Clipped: share of pixels the DNG can't hold
//! POST /api/settings {file, settings, adjust_only}  remember a frame's settings (its sidecar)
//! POST /api/export   {files}               save DNGs in the background, each with its own settings
//! GET  /api/export/status                  progress of that
//! POST /api/export/cancel                  frames not started yet don't start
//! POST /api/roll     {files}               read each frame's film base, changing nothing; runs
//!                                          like an export, with the same progress
//! GET  /api/roll/readings                  what that read
//! POST /api/roll/vote {bases, readings}    the roll's base from `bases`, and which readings
//!                                          stray from it
//! POST /api/roll/apply {files, bases}      use the base from `bases` on `files`; nothing is
//!                                          written unless all of them can be read

use std::collections::HashMap;
use std::io::Cursor;
use std::panic::{catch_unwind, AssertUnwindSafe};
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

use crate::batch::{lock, Batch, Done};
use crate::folder::RollReading;
use crate::folder::{saved_settings, Folder};

const INDEX_HTML: &str = include_str!("../ui/index.html");
const APP_JS: &str = include_str!("../ui/app.js");
const STYLE_CSS: &str = include_str!("../ui/style.css");
const LICENSE_TEXT: &str = include_str!("../../../LICENSE");

/// What the page may load and connect to: its own script and stylesheet, previews as blobs,
/// icons as data URLs, and the API. Nothing inline, so a file name or message that reached the
/// page as markup couldn't run as code, and no site may put the page in a frame.
const CONTENT_SECURITY_POLICY: &str = "default-src 'none'; script-src 'self'; style-src 'self'; \
    img-src 'self' blob: data:; connect-src 'self'; frame-ancestors 'none'; base-uri 'none'; \
    form-action 'none'";

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
    roll_readings: Arc<Mutex<Vec<RollReading>>>,
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
    /// Only the adjustments count: a roll is rewriting the measurements.
    #[serde(default)]
    adjust_only: bool,
}

#[derive(Deserialize)]
struct ExportJob {
    files: Vec<String>,
}

#[derive(Deserialize)]
struct RollVote {
    /// The frames that count.
    bases: Vec<[f32; 3]>,
    /// All frames' readings, each compared with the result.
    readings: Vec<[f32; 3]>,
}

#[derive(Deserialize)]
struct RollApply {
    files: Vec<String>,
    bases: Vec<[f32; 3]>,
}

#[derive(Deserialize)]
struct HolderJob {
    file: String,
    base_rgb: [f32; 3],
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
        roll_readings: Arc::default(),
    };
    for mut req in server.incoming_requests() {
        // A failing request mustn't take the server down: the Mac app would close with it.
        let handled = catch_unwind(AssertUnwindSafe(|| state.handle(&mut req)));
        let resp = match handled {
            Ok(Ok(resp)) => resp,
            Ok(Err((code, msg))) => json_resp(code, &json!({ "error": msg })),
            Err(_) => {
                eprintln!("{} {} failed unexpectedly", req.method(), req.url());
                json_resp(500, &json!({ "error": "the request failed unexpectedly" }))
            }
        };
        // Browsers must take each response for what it says it is.
        let _ = req.respond(resp.with_header(header("X-Content-Type-Options", "nosniff")));
    }
    Ok(())
}

fn header(name: &str, value: &str) -> Header {
    Header::from_bytes(name.as_bytes(), value.as_bytes()).expect("static header")
}

fn version_info() -> serde_json::Value {
    serde_json::json!({ "version": env!("CARGO_PKG_VERSION"), "commit": env!("REVERSAL_COMMIT") })
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

fn request_header(req: &Request, name: &str) -> Option<String> {
    req.headers()
        .iter()
        .find(|h| h.field.as_str().as_str().eq_ignore_ascii_case(name))
        .map(|h| h.value.to_string())
}

/// Binding to loopback doesn't stop a web page in the user's browser from reaching this
/// server. The Host check defeats DNS rebinding, the Origin check other sites, and
/// requiring JSON forces a CORS preflight, which this server never grants. The Sec-Fetch
/// headers, where a browser sends them, also refuse what another site loads without a
/// preflight, such as an image; a link from another site may still open the page itself.
fn request_allowed(
    port: u16,
    is_post: bool,
    is_page: bool,
    header: impl Fn(&str) -> Option<String>,
) -> bool {
    let local = |authority: &str| {
        authority == format!("127.0.0.1:{port}") || authority == format!("localhost:{port}")
    };
    let is_json = |c: &str| {
        let media_type = c.split(';').next().unwrap_or("").trim();
        media_type.eq_ignore_ascii_case("application/json")
    };
    let own_fetch = header("Sec-Fetch-Site").is_none_or(|s| s == "same-origin" || s == "none");
    let navigation = is_page
        && header("Sec-Fetch-Mode").as_deref() == Some("navigate")
        && header("Sec-Fetch-Dest").as_deref() == Some("document");
    header("Host").is_some_and(|h| local(&h))
        && header("Origin").is_none_or(|o| o.strip_prefix("http://").is_some_and(local))
        && (!is_post || header("Content-Type").is_some_and(|c| is_json(&c)))
        && (own_fetch || navigation)
}

fn page(body: &str, content_type: &str) -> Resp {
    Response::from_data(body.as_bytes().to_vec())
        .with_header(header("Content-Type", content_type))
        .with_header(header("Content-Security-Policy", CONTENT_SECURITY_POLICY))
        .with_header(header("Referrer-Policy", "no-referrer"))
        // The files change with the binary, not with time, so never keep an old one.
        .with_header(header("Cache-Control", "no-store"))
}

fn read_body<T: for<'de> Deserialize<'de>>(req: &mut Request) -> Result<T, (u16, String)> {
    serde_json::from_reader(req.as_reader()).map_err(|e| (400, format!("bad request body: {e}")))
}

impl State {
    fn handle(&mut self, req: &mut Request) -> Handled {
        let url = req.url().to_string();
        let path = url.split('?').next().unwrap_or("");
        let is_page = matches!(path, "/" | "/app.js" | "/style.css");
        let allowed = request_allowed(self.port, *req.method() == Method::Post, is_page, |name| {
            request_header(req, name)
        });
        if !allowed {
            return Err((403, "request refused".to_string()));
        }
        match (req.method(), path) {
            (Method::Get, "/") => Ok(page(INDEX_HTML, "text/html; charset=utf-8")),
            (Method::Get, "/app.js") => Ok(page(APP_JS, "text/javascript; charset=utf-8")),
            (Method::Get, "/style.css") => Ok(page(STYLE_CSS, "text/css; charset=utf-8")),
            (Method::Get, "/api/version") => Ok(json_resp(200, &version_info())),
            (Method::Get, "/api/license") => {
                Ok(Response::from_data(LICENSE_TEXT.as_bytes().to_vec())
                    .with_header(header("Content-Type", "text/plain; charset=utf-8")))
            }
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
            (Method::Post, "/api/holder") => {
                let job: HolderJob = read_body(req)?;
                let loaded = self.ensure_loaded(&job.file)?;
                let areas = invert::holder_areas(&loaded.small, job.base_rgb);
                Ok(json_resp(200, &json!({ "areas": areas })))
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
                self.read_roll(job.files)
            }
            (Method::Get, "/api/roll/readings") => {
                let readings = lock(&self.roll_readings);
                Ok(json_resp(200, &json!(*readings)))
            }
            (Method::Post, "/api/roll/vote") => {
                let vote: RollVote = read_body(req)?;
                let base_rgb = voted_base(&vote.bases)?;
                let differs: Vec<bool> = vote
                    .readings
                    .iter()
                    .map(|&base| invert::differs_from_roll(base, base_rgb))
                    .collect();
                Ok(json_resp(
                    200,
                    &json!({ "base_rgb": base_rgb, "differs": differs }),
                ))
            }
            (Method::Post, "/api/roll/apply") => {
                let job: RollApply = read_body(req)?;
                self.apply_roll(job)
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
        self.ensure_loaded(file)?;
        let sidecar = std::fs::read(self.folder.sidecar(file));
        let loaded = &self.cache[file];
        let auto = loaded.auto.clone();
        let base_found = invert::detect_base(&loaded.small).found;
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
                "baseFound": base_found,
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
        let saved = if job.adjust_only {
            self.folder.save_adjustments(&job.file, &job.settings)
        } else {
            self.folder.save_settings(&job.file, &job.settings)
        };
        saved.map_err(|e| (500, e.to_string()))?;
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
    fn read_roll(&self, files: Vec<String>) -> Handled {
        if files.len() < 2 {
            return Err((400, "a roll needs at least two frames".to_string()));
        }
        for file in &files {
            self.folder.resolve(file)?;
        }
        if self.batch.is_running() {
            return Err((409, "still saving the previous DNGs".to_string()));
        }
        lock(&self.roll_readings).clear();
        let (folder, readings) = (self.folder.clone(), self.roll_readings.clone());
        let started = self.batch.start_then(
            "roll-read",
            files,
            export_workers(),
            move |file| {
                let reading = folder.read_roll_base(file)?;
                lock(&readings).push(reading);
                Ok(Done {
                    path: String::new(),
                    warning: None,
                })
            },
            || None,
        );
        started.map_err(|e| (409, e))?;
        Ok(json_resp(200, &self.batch.status()))
    }

    fn apply_roll(&self, job: RollApply) -> Handled {
        let base_rgb = voted_base(&job.bases)?;
        for file in &job.files {
            self.folder.resolve(file)?;
        }
        let prepared = Arc::new(Mutex::new(Vec::with_capacity(job.files.len())));
        let total = job.files.len();
        let (folder, collected) = (self.folder.clone(), prepared.clone());
        let apply_folder = self.folder.clone();
        self.batch
            .start_then(
                "roll",
                job.files,
                1,
                move |file| {
                    let frame = folder.prepare_roll_frame(file)?;
                    lock(&collected).push(frame);
                    Ok(Done {
                        path: String::new(),
                        warning: None,
                    })
                },
                move || {
                    let frames = std::mem::take(&mut *lock(&prepared));
                    if frames.len() < total {
                        return Some(
                            "Not every frame could be read, so nothing changed.".to_string(),
                        );
                    }
                    apply_folder.apply_roll_base(&frames, base_rgb).err()
                },
            )
            .map_err(|e| (409, e))?;
        Ok(json_resp(200, &self.batch.status()))
    }
}

/// The roll's film base from the frames chosen to count.
fn voted_base(bases: &[[f32; 3]]) -> Result<[f32; 3], (u16, String)> {
    if bases.is_empty() {
        return Err((400, "choose at least one frame to measure from".to_string()));
    }
    Ok(invert::roll_base(bases))
}

/// Frames saved at once. Each also uses all cores while converting, and holds a few hundred MB.
fn export_workers() -> usize {
    let cores = std::thread::available_parallelism().map_or(1, |n| n.get());
    (cores / 3).clamp(1, 4)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A request for an API path with these headers.
    fn with_headers(is_post: bool, headers: &[(&str, &str)]) -> bool {
        request_allowed(8787, is_post, false, |name| {
            headers
                .iter()
                .find(|(n, _)| n.eq_ignore_ascii_case(name))
                .map(|(_, v)| v.to_string())
        })
    }

    fn allowed(host: &str, origin: Option<&str>, content_type: Option<&str>) -> bool {
        let mut headers = vec![("Host", host)];
        headers.extend(origin.map(|o| ("Origin", o)));
        headers.extend(content_type.map(|c| ("Content-Type", c)));
        with_headers(true, &headers)
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
        assert!(with_headers(false, &[("Host", "localhost:8787")]));
        assert!(!with_headers(false, &[]));
    }

    #[test]
    fn another_site_cant_load_the_api_but_the_user_can_still_go_to_a_page() {
        let host = ("Host", "127.0.0.1:8787");
        // The page's own calls, and the user typing the address or using a bookmark.
        assert!(with_headers(
            false,
            &[host, ("Sec-Fetch-Site", "same-origin")]
        ));
        assert!(with_headers(false, &[host, ("Sec-Fetch-Site", "none")]));
        // An image or script tag on another site, which needs no preflight.
        let image = [
            host,
            ("Sec-Fetch-Site", "cross-site"),
            ("Sec-Fetch-Mode", "no-cors"),
            ("Sec-Fetch-Dest", "image"),
        ];
        assert!(!with_headers(false, &image));
        // Another local server, say on another port.
        assert!(!with_headers(
            false,
            &[host, ("Sec-Fetch-Site", "same-site")]
        ));
        // A frame on another site: refused here and by the policy on the page.
        let frame = [
            host,
            ("Sec-Fetch-Site", "cross-site"),
            ("Sec-Fetch-Mode", "navigate"),
            ("Sec-Fetch-Dest", "iframe"),
        ];
        assert!(!with_headers(false, &frame));
        // A link from another site opens the page: the user sees it, not that site. A link to
        // an API path doesn't, so no site can make the server decode a file.
        let link = [
            host,
            ("Sec-Fetch-Site", "cross-site"),
            ("Sec-Fetch-Mode", "navigate"),
            ("Sec-Fetch-Dest", "document"),
        ];
        assert!(!with_headers(false, &link));
        let find = |name: &str| {
            link.iter()
                .find(|(n, _)| n.eq_ignore_ascii_case(name))
                .map(|(_, v)| v.to_string())
        };
        assert!(request_allowed(8787, false, true, find));
    }

    #[test]
    fn the_page_keeps_its_code_in_files_so_the_policy_holds() {
        // No inline script or style: the policy would block it, silently in a user's browser.
        assert_eq!(INDEX_HTML.matches("<script").count(), 1);
        assert!(INDEX_HTML.contains(r#"<script src="/app.js"></script>"#));
        assert!(INDEX_HTML.contains(r#"<link rel="stylesheet" href="/style.css">"#));
        assert!(!INDEX_HTML.contains("<style"));
        for text in [INDEX_HTML, APP_JS] {
            assert!(!text.contains("style=\""), "inline style attribute");
            // Event handler attributes, such as onclick="...".
            let handler = text.split(" on").skip(1).any(|rest| {
                let name: String = rest.chars().take_while(char::is_ascii_lowercase).collect();
                !name.is_empty() && rest[name.len()..].starts_with("=\"")
            });
            assert!(!handler, "inline event handler");
        }
        for part in CONTENT_SECURITY_POLICY.split(';') {
            assert!(!part.contains("unsafe"), "{part}");
        }
    }

    #[test]
    fn version_comes_from_the_workspace_and_the_license_is_embedded() {
        let info = version_info();
        assert_eq!(info["version"], env!("CARGO_PKG_VERSION"));
        assert_eq!(info["version"].as_str().unwrap().split('.').count(), 3);
        assert!(LICENSE_TEXT.contains("GNU GENERAL PUBLIC LICENSE"));
    }

    #[test]
    fn the_mac_app_takes_its_version_from_the_build() {
        let plist = include_str!("../../../packaging/macos/Info.plist");
        assert!(plist.contains("<key>CFBundleShortVersionString</key><string>@VERSION@</string>"));
        assert!(plist.contains("<key>CFBundleVersion</key><string>@BUILD@</string>"));
    }

    #[test]
    fn notices_name_libraw_and_adobe_wherever_a_copyright_shows() {
        let plist = include_str!("../../../packaging/macos/Info.plist");
        let read_me = include_str!("../../../packaging/macos/Read me first.txt");
        for text in [INDEX_HTML, plist, read_me] {
            assert!(text.contains("2008-2025 LibRaw LLC"));
            assert!(text.contains("under license by Adobe"));
        }
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
