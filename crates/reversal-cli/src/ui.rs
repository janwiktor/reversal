// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Local web UI: one page and a JSON API, served by a small HTTP server on 127.0.0.1, or by
//! an [`Engine`] that an app holds in its own process and passes its web view's requests to.
//! Requests from other sites are refused, see [`request_allowed`], and the page runs under a
//! content security policy that allows only its own files, see [`CONTENT_SECURITY_POLICY`].
//!
//! GET  /, /app.js, /style.css             the page
//! GET  /api/version                        version and the commit it was built from
//! GET  /api/license                        the license text
//! GET  /api/files                          raw files in the folder
//! GET  /api/load?file=NAME                 decode (cached), auto and saved settings, warning,
//!                                          and the film type the frame reads as
//! POST /api/estimate {file, hints, straighten, film}   re-run the analysis with manual choices,
//!                                          converting as `film` unless the hints choose
//! POST /api/holder   {file, base_rgb, straighten}   where the analysis ignores the film holder
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
use std::io::{Cursor, Read};
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
use tiny_http::{Header, Server};

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

/// The page's origin when an app serves it to its own web view, without a server. The app
/// registers this scheme there and passes each request to [`Engine::respond`].
pub const APP_ORIGIN: &str = "reversal://app";

/// Bodies are settings and lists of file names. Anything larger isn't from the page.
const MAX_BODY_BYTES: usize = 4 << 20;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Method {
    Get,
    Post,
    Other,
}

/// A request from the page, whether it came over HTTP or from an app's own web view.
pub struct Request {
    pub method: Method,
    /// The path and query.
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Request {
    fn header(&self, name: &str) -> Option<String> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.clone())
    }
}

pub struct Response {
    pub status: u16,
    pub headers: Vec<(&'static str, String)>,
    pub body: Vec<u8>,
}

impl Response {
    fn new(content_type: &str, body: Vec<u8>) -> Self {
        Response {
            status: 200,
            headers: vec![("Content-Type", content_type.to_string())],
            body,
        }
    }

    fn with_header(mut self, name: &'static str, value: &str) -> Self {
        self.headers.push((name, value.to_string()));
        self
    }
}

type Handled = Result<Response, (u16, String)>;

/// How the page is served, which decides where its requests have to come from.
#[derive(Clone, Copy)]
enum Served {
    Http {
        port: u16,
    },
    /// By an app, to its own web view, as [`APP_ORIGIN`].
    App,
}

struct Loaded {
    /// Downsampled scan: used for previews and (re-)estimation.
    small: Rgb32,
    auto: Settings,
}

struct State {
    served: Served,
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
    #[serde(default)]
    straighten: f32,
}

#[derive(Deserialize)]
struct EstimateJob {
    file: String,
    hints: Hints,
    #[serde(default)]
    straighten: f32,
    /// The film type the frame converts as now, which measuring again keeps unless the hints
    /// choose one. Without it, the detected type.
    #[serde(default)]
    film: Option<invert::Film>,
}

/// Serve `dir` on `port`, or on a free port if it's 0. With `exit_with_stdin` the process ends
/// when stdin closes, so it can't outlive the app that started it.
pub fn serve(
    dir: PathBuf,
    port: u16,
    exit_with_stdin: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let (server, mut state, port) = bind(dir, port)?;
    if exit_with_stdin {
        std::thread::spawn(|| {
            let _ = std::io::copy(&mut std::io::stdin(), &mut std::io::sink());
            std::process::exit(0);
        });
    }
    println!(
        "Reversal UI: http://127.0.0.1:{port}  (folder: {})",
        state.folder.dir.display()
    );
    state.run(&server);
    Ok(())
}

/// The engine for an app that holds it in its own process and passes it the requests of its web
/// view, where an app can't start another process and has no reason to open a port. The page is
/// the one the server serves, at [`APP_ORIGIN`].
pub struct Engine {
    state: State,
}

impl Engine {
    pub fn open(dir: PathBuf) -> std::io::Result<Self> {
        let state = State::new(dir.canonicalize()?, Served::App);
        Ok(Engine { state })
    }

    /// Answers one request. A save it started runs in the background.
    pub fn respond(&mut self, request: &Request) -> Response {
        self.state.respond(request)
    }
}

/// Closing a folder stops its background work: the frames in progress finish, and the rest
/// don't start. Nobody would see them, and the next folder's work would run beside them.
impl Drop for Engine {
    fn drop(&mut self) {
        self.state.batch.cancel();
    }
}

fn bind(dir: PathBuf, port: u16) -> Result<(Server, State, u16), Box<dyn std::error::Error>> {
    let dir = dir.canonicalize()?;
    let server = Server::http(("127.0.0.1", port))
        .map_err(|e| format!("could not use port {port}: {e}. Choose another with --port."))?;
    let port = server
        .server_addr()
        .to_ip()
        .map_or(port, |address| address.port());
    Ok((server, State::new(dir, Served::Http { port }), port))
}

/// The request as the engine takes it, or the answer if it can't be read.
fn read_http(req: &mut tiny_http::Request) -> Result<Request, Response> {
    let method = match req.method() {
        tiny_http::Method::Get => Method::Get,
        tiny_http::Method::Post => Method::Post,
        _ => Method::Other,
    };
    let headers = req
        .headers()
        .iter()
        .map(|h| (h.field.as_str().to_string(), h.value.to_string()))
        .collect();
    let mut body = Vec::new();
    if method == Method::Post {
        let limit = MAX_BODY_BYTES as u64 + 1;
        let read = req.as_reader().take(limit).read_to_end(&mut body);
        if read.is_err() || body.len() > MAX_BODY_BYTES {
            return Err(json_resp(413, &json!({ "error": "request too large" })));
        }
    }
    Ok(Request {
        method,
        url: req.url().to_string(),
        headers,
        body,
    })
}

fn to_http(response: Response) -> tiny_http::Response<Cursor<Vec<u8>>> {
    let mut http = tiny_http::Response::from_data(response.body).with_status_code(response.status);
    for (name, value) in response.headers {
        if let Ok(header) = Header::from_bytes(name.as_bytes(), value.as_bytes()) {
            http = http.with_header(header);
        }
    }
    http
}

/// Browsers must take each response for what it says it is.
fn finish(response: Response) -> Response {
    response.with_header("X-Content-Type-Options", "nosniff")
}

fn version_info() -> serde_json::Value {
    serde_json::json!({ "version": env!("CARGO_PKG_VERSION"), "commit": env!("REVERSAL_COMMIT") })
}

fn json_resp(code: u16, v: &serde_json::Value) -> Response {
    Response {
        status: code,
        ..Response::new("application/json", v.to_string().into_bytes())
    }
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

/// Binding to loopback doesn't stop a web page in the user's browser from reaching this
/// server. The Host check defeats DNS rebinding, the Origin check other sites, and
/// requiring JSON forces a CORS preflight, which this server never grants. The Sec-Fetch
/// headers, where a browser sends them, also refuse what another site loads without a
/// preflight, such as an image; a link from another site may still open the page itself.
///
/// An app's web view reaches the engine without a network, so there is no host to check. The
/// origin still has to be the page's, should that view ever show anything else.
fn request_allowed(
    served: Served,
    is_post: bool,
    is_page: bool,
    header: impl Fn(&str) -> Option<String>,
) -> bool {
    let from_page = match served {
        Served::Http { port } => {
            let local = |authority: &str| {
                authority == format!("127.0.0.1:{port}") || authority == format!("localhost:{port}")
            };
            header("Host").is_some_and(|h| local(&h))
                && header("Origin").is_none_or(|o| o.strip_prefix("http://").is_some_and(local))
        }
        Served::App => header("Origin").is_none_or(|o| o == APP_ORIGIN),
    };
    let is_json = |c: &str| {
        let media_type = c.split(';').next().unwrap_or("").trim();
        media_type.eq_ignore_ascii_case("application/json")
    };
    let own_fetch = header("Sec-Fetch-Site").is_none_or(|s| s == "same-origin" || s == "none");
    let navigation = is_page
        && header("Sec-Fetch-Mode").as_deref() == Some("navigate")
        && header("Sec-Fetch-Dest").as_deref() == Some("document");
    from_page
        && (!is_post || header("Content-Type").is_some_and(|c| is_json(&c)))
        && (own_fetch || navigation)
}

fn page(body: &str, content_type: &str) -> Response {
    Response::new(content_type, body.as_bytes().to_vec())
        .with_header("Content-Security-Policy", CONTENT_SECURITY_POLICY)
        .with_header("Referrer-Policy", "no-referrer")
        // The files change with the binary, not with time, so never keep an old one.
        .with_header("Cache-Control", "no-store")
}

fn read_body<T: for<'de> Deserialize<'de>>(req: &Request) -> Result<T, (u16, String)> {
    serde_json::from_slice(&req.body).map_err(|e| (400, format!("bad request body: {e}")))
}

impl State {
    /// `dir` as `canonicalize` gives it.
    fn new(dir: PathBuf, served: Served) -> Self {
        State {
            served,
            folder: Arc::new(Folder::new(dir)),
            cache: HashMap::new(),
            batch: Batch::default(),
            roll_readings: Arc::default(),
        }
    }

    fn run(&mut self, server: &Server) {
        for mut req in server.incoming_requests() {
            let response = match read_http(&mut req) {
                Ok(request) => self.respond(&request),
                Err(refused) => finish(refused),
            };
            let _ = req.respond(to_http(response));
        }
    }

    fn respond(&mut self, req: &Request) -> Response {
        // A failing request mustn't take the engine down: the app would close with it.
        let handled = catch_unwind(AssertUnwindSafe(|| self.handle(req)));
        finish(match handled {
            Ok(Ok(response)) => response,
            Ok(Err((code, msg))) => json_resp(code, &json!({ "error": msg })),
            Err(_) => {
                eprintln!("{:?} {} failed unexpectedly", req.method, req.url);
                json_resp(500, &json!({ "error": "the request failed unexpectedly" }))
            }
        })
    }

    fn handle(&mut self, req: &Request) -> Handled {
        let url = req.url.as_str();
        let path = url.split('?').next().unwrap_or("");
        let is_page = matches!(path, "/" | "/app.js" | "/style.css");
        let is_post = req.method == Method::Post;
        if !request_allowed(self.served, is_post, is_page, |name| req.header(name)) {
            return Err((403, "request refused".to_string()));
        }
        match (req.method, path) {
            (Method::Get, "/") => Ok(page(INDEX_HTML, "text/html; charset=utf-8")),
            (Method::Get, "/app.js") => Ok(page(APP_JS, "text/javascript; charset=utf-8")),
            (Method::Get, "/style.css") => Ok(page(STYLE_CSS, "text/css; charset=utf-8")),
            (Method::Get, "/api/version") => Ok(json_resp(200, &version_info())),
            (Method::Get, "/api/license") => Ok(Response::new(
                "text/plain; charset=utf-8",
                LICENSE_TEXT.as_bytes().to_vec(),
            )),
            (Method::Get, "/api/files") => self.files(),
            (Method::Get, "/api/load") => {
                let file = query_param(url, "file").ok_or((400, "missing file".to_string()))?;
                self.load(&file)
            }
            (Method::Post, "/api/estimate") => {
                let job: EstimateJob = read_body(req)?;
                let loaded = self.ensure_loaded(&job.file)?;
                // Areas are on the straightened scan, so that's what gets measured.
                let small = loaded.small.straighten(job.straighten);
                let (estimate, reading) = invert::measure_frame(&small, &job.hints, job.film);
                Ok(json_resp(
                    200,
                    &json!({
                        "estimate": estimate,
                        "filmDetected": reading.film,
                        "baseFound": reading.base_found,
                    }),
                ))
            }
            (Method::Post, "/api/holder") => {
                let job: HolderJob = read_body(req)?;
                let loaded = self.ensure_loaded(&job.file)?;
                let small = loaded.small.straighten(job.straighten);
                let areas = invert::holder_areas(&small, job.base_rgb);
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
        let (current, warning) = match sidecar {
            Ok(bytes) => saved_settings(&bytes, &auto, &loaded.small),
            Err(_) => (auto.clone(), None),
        };
        let straight = loaded.small.straighten(current.adjust.straighten);
        let reading = invert::read_frame(&straight, &current.hints);
        Ok(json_resp(
            200,
            &json!({
                "filmDetected": reading.film,
                "auto": auto,
                "settings": current,
                "warning": warning,
                "defaultAreaPercent": invert::DEFAULT_AREA_PERCENT,
                "baseFound": reading.base_found,
            }),
        ))
    }

    fn preview(&mut self, job: &Job) -> Handled {
        let loaded = self.ensure_loaded(&job.file)?;
        let full = job.full.then_some(NRect::FULL);
        let img = invert::convert_with(&loaded.small, &job.settings, full);
        let clipped =
            dng::clipped_share(&invert::picture_for_clipping(&loaded.small, &job.settings));
        let (profile, _) = reversal_core::profiles(&job.settings);
        let jpeg = render::preview_jpeg(&img, &profile, 88);
        Ok(Response::new("image/jpeg", jpeg)
            .with_header("Cache-Control", "no-store")
            .with_header("X-Clipped", &format!("{clipped:.6}")))
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
            return Err((409, "another task is still running".to_string()));
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
                        return Some("Could not read every frame, so nothing changed".to_string());
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
        return Err((400, "select at least one frame".to_string()));
    }
    Ok(invert::roll_base(bases))
}

/// Frames saved at once. Each also uses all cores while converting, and holds a few hundred MB,
/// which on iOS would get the app killed sooner than it would save time.
fn export_workers() -> usize {
    if cfg!(target_os = "ios") {
        return 1;
    }
    let cores = std::thread::available_parallelism().map_or(1, |n| n.get());
    (cores / 3).clamp(1, 4)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A request for an API path with these headers.
    fn with_headers(is_post: bool, headers: &[(&str, &str)]) -> bool {
        request_allowed(Served::Http { port: 8787 }, is_post, false, |name| {
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
        assert!(request_allowed(
            Served::Http { port: 8787 },
            false,
            true,
            find
        ));
    }

    fn request(method: Method, url: &str, headers: &[(&str, &str)], body: &str) -> Request {
        Request {
            method,
            url: url.to_string(),
            headers: headers
                .iter()
                .map(|(n, v)| (n.to_string(), v.to_string()))
                .collect(),
            body: body.as_bytes().to_vec(),
        }
    }

    fn header_of<'a>(response: &'a Response, name: &str) -> Option<&'a str> {
        let found = response.headers.iter().find(|(n, _)| *n == name);
        found.map(|(_, value)| value.as_str())
    }

    #[test]
    fn an_app_gets_the_page_and_the_api_without_a_server() {
        let dir = std::env::temp_dir().join(format!("reversal-engine-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut engine = Engine::open(dir.clone()).unwrap();

        let page = engine.respond(&request(Method::Get, "/", &[], ""));
        assert_eq!(page.status, 200);
        assert_eq!(page.body, INDEX_HTML.as_bytes());
        assert_eq!(
            header_of(&page, "Content-Security-Policy"),
            Some(CONTENT_SECURITY_POLICY)
        );
        assert_eq!(header_of(&page, "X-Content-Type-Options"), Some("nosniff"));

        let files = engine.respond(&request(Method::Get, "/api/files", &[], ""));
        let listed: serde_json::Value = serde_json::from_slice(&files.body).unwrap();
        assert_eq!(listed["files"], json!([]));

        // The page's own calls carry its origin, and a POST is JSON.
        let json = ("Content-Type", "application/json");
        let vote = r#"{"bases":[[0.5,0.4,0.3]],"readings":[[0.5,0.4,0.3]]}"#;
        let own = [json, ("Origin", APP_ORIGIN)];
        let voted = engine.respond(&request(Method::Post, "/api/roll/vote", &own, vote));
        assert_eq!(voted.status, 200);
        let other = [json, ("Origin", "https://evil.example")];
        let refused = engine.respond(&request(Method::Post, "/api/roll/vote", &other, vote));
        assert_eq!(refused.status, 403);
        let form = engine.respond(&request(Method::Post, "/api/roll/vote", &[], vote));
        assert_eq!(form.status, 403);

        // Mistakes come back as JSON, not as a panic: a bad body, a path or a method unknown.
        let bad = engine.respond(&request(Method::Post, "/api/roll/vote", &own, "{"));
        assert_eq!(bad.status, 400);
        let missing = engine.respond(&request(Method::Get, "/api/nothing", &[], ""));
        assert_eq!(missing.status, 404);
        assert_eq!(
            header_of(&missing, "Content-Type"),
            Some("application/json")
        );
        let put = engine.respond(&request(Method::Other, "/api/files", &[], ""));
        assert_eq!(put.status, 404);
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// One exchange with the server over a socket: (status line, headers and body).
    fn exchange(port: u16, request: &[u8]) -> String {
        use std::io::Write;
        let mut stream = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
        // The server may answer and close before it has read all of a body that's too large.
        let _ = stream.write_all(request);
        let mut reply = Vec::new();
        let _ = stream.read_to_end(&mut reply);
        String::from_utf8_lossy(&reply).into_owned()
    }

    #[test]
    fn the_server_carries_requests_and_answers_across_http() {
        let dir = std::env::temp_dir().join(format!("reversal-http-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (server, mut state, port) = bind(dir.clone(), 0).unwrap();
        let server = Arc::new(server);
        let serving = server.clone();
        let thread = std::thread::spawn(move || state.run(&serving));
        let host = format!("Host: 127.0.0.1:{port}\r\nConnection: close\r\n");

        let version = exchange(
            port,
            format!("GET /api/version HTTP/1.1\r\n{host}\r\n").as_bytes(),
        );
        assert!(version.starts_with("HTTP/1.1 200"), "{version}");
        assert!(version.contains("Content-Type: application/json"));
        assert!(version.contains("X-Content-Type-Options: nosniff"));
        assert!(version.contains(env!("CARGO_PKG_VERSION")));

        // A POST's headers and body arrive, and an error keeps its status and JSON shape.
        let post = |body: &[u8]| {
            let head = format!(
                "POST /api/roll/vote HTTP/1.1\r\n{host}Content-Type: application/json\r\nContent-Length: {}\r\n\r\n",
                body.len()
            );
            exchange(port, &[head.as_bytes(), body].concat())
        };
        let voted = post(br#"{"bases":[[0.5,0.4,0.3]],"readings":[]}"#);
        assert!(voted.starts_with("HTTP/1.1 200"), "{voted}");
        assert!(voted.contains("base_rgb"));
        let empty = post(br#"{"bases":[],"readings":[]}"#);
        assert!(empty.starts_with("HTTP/1.1 400"), "{empty}");
        assert!(empty.contains(r#"{"error":"select at least one frame"}"#));
        let large = post(&vec![b' '; MAX_BODY_BYTES + 1]);
        assert!(large.starts_with("HTTP/1.1 413"), "{large}");

        server.unblock();
        thread.join().unwrap();
        std::fs::remove_dir_all(dir).unwrap();
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
        assert!(LICENSE_TEXT.contains("Mozilla Public License Version 2.0"));
    }

    #[test]
    fn the_mac_app_takes_its_version_from_the_build() {
        let plist = include_str!("../../../packaging/macos/Info.plist");
        assert!(plist.contains("<key>CFBundleShortVersionString</key><string>@VERSION@</string>"));
        assert!(plist.contains("<key>CFBundleVersion</key><string>@BUILD@</string>"));
    }

    #[test]
    fn notices_name_the_licenses_wherever_a_copyright_shows() {
        let plist = include_str!("../../../packaging/macos/Info.plist");
        let read_me = include_str!("../../../packaging/macos/Read me first.txt");
        let ios_plist = include_str!("../../../packaging/ios/Info.plist");
        for text in [INDEX_HTML, plist, read_me, ios_plist] {
            assert!(text.contains("Mozilla Public License 2.0"));
            assert!(text.contains("CDDL 1.0"));
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
