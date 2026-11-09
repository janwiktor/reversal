// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! C interface for the iOS app. An iOS app can't start the `reversal` binary, so it holds the
//! engine itself and passes it the requests of its web view. See `packaging/ios/Engine.h`.

use std::ffi::{c_char, CStr, CString};
use std::path::PathBuf;
use std::sync::Mutex;

use reversal_cli::batch::lock;
use reversal_cli::folder::is_scan_name;
use reversal_cli::ui::{Engine, Method, Request, Response};

/// One folder at a time, as with the Mac app's engine process.
static ENGINE: Mutex<Option<Engine>> = Mutex::new(None);

/// An answer to [`reversal_engine_request`], to give back to [`reversal_response_free`].
#[repr(C)]
pub struct ReversalResponse {
    pub status: u16,
    /// One `Name: value` per line.
    pub headers: *mut c_char,
    pub body: *mut u8,
    pub body_len: usize,
}

/// Open `folder` in place of the one that was open. Returns whether that worked; if not, no
/// folder is open.
///
/// # Safety
/// `folder` must be a NUL-terminated string.
#[no_mangle]
pub unsafe extern "C" fn reversal_engine_open(folder: *const c_char) -> bool {
    let mut engine = lock(&ENGINE);
    *engine = None;
    let Ok(folder) = CStr::from_ptr(folder).to_str() else {
        return false;
    };
    match Engine::open(PathBuf::from(folder)) {
        Ok(opened) => {
            *engine = Some(opened);
            true
        }
        Err(e) => {
            eprintln!("could not open {folder}: {e}");
            false
        }
    }
}

/// Close the folder. A save in progress finishes first.
#[no_mangle]
pub extern "C" fn reversal_engine_close() {
    *lock(&ENGINE) = None;
}

/// Answer one request from the page: `method` is GET or POST, `url` the path and query, and
/// `headers` one `Name: value` per line. It waits for a request that's being answered, since the
/// engine takes one at a time. Never returns null.
///
/// # Safety
/// `method`, `url` and `headers` must be NUL-terminated strings, and `body` must point to
/// `body_len` bytes, or be null with a length of 0.
#[no_mangle]
pub unsafe extern "C" fn reversal_engine_request(
    method: *const c_char,
    url: *const c_char,
    headers: *const c_char,
    body: *const u8,
    body_len: usize,
) -> *mut ReversalResponse {
    let text = |pointer| CStr::from_ptr(pointer).to_string_lossy().into_owned();
    let body = if body.is_null() {
        Vec::new()
    } else {
        std::slice::from_raw_parts(body, body_len).to_vec()
    };
    let request = Request {
        method: match text(method).as_str() {
            "GET" => Method::Get,
            "POST" => Method::Post,
            _ => Method::Other,
        },
        url: text(url),
        headers: header_lines(&text(headers)),
        body,
    };
    into_c(respond(&request))
}

fn respond(request: &Request) -> Response {
    match lock(&ENGINE).as_mut() {
        Some(engine) => engine.respond(request),
        None => Response {
            status: 503,
            headers: vec![("Content-Type", "application/json".to_string())],
            body: br#"{"error":"no folder is open"}"#.to_vec(),
        },
    }
}

fn header_lines(lines: &str) -> Vec<(String, String)> {
    lines
        .lines()
        .filter_map(|line| line.split_once(':'))
        .map(|(name, value)| (name.trim().to_string(), value.trim().to_string()))
        .collect()
}

fn into_c(response: Response) -> *mut ReversalResponse {
    let lines: String = response
        .headers
        .iter()
        .map(|(name, value)| format!("{name}: {}\n", value.replace(['\r', '\n'], " ")))
        .collect();
    // The engine writes these headers itself, and none holds a NUL.
    let headers = CString::new(lines).unwrap_or_default();
    let body = response.body.into_boxed_slice();
    let body_len = body.len();
    Box::into_raw(Box::new(ReversalResponse {
        status: response.status,
        headers: headers.into_raw(),
        body: Box::into_raw(body).cast::<u8>(),
        body_len,
    }))
}

/// Free an answer from [`reversal_engine_request`].
///
/// # Safety
/// `response` must come from [`reversal_engine_request`], and must not be used again.
#[no_mangle]
pub unsafe extern "C" fn reversal_response_free(response: *mut ReversalResponse) {
    if response.is_null() {
        return;
    }
    let response = Box::from_raw(response);
    drop(CString::from_raw(response.headers));
    let body = std::ptr::slice_from_raw_parts_mut(response.body, response.body_len);
    drop(Box::from_raw(body));
}

/// Whether the engine would list a file of this name as a frame.
///
/// # Safety
/// `name` must be a NUL-terminated string.
#[no_mangle]
pub unsafe extern "C" fn reversal_is_scan(name: *const c_char) -> bool {
    CStr::from_ptr(name).to_str().is_ok_and(is_scan_name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use reversal_cli::ui::APP_ORIGIN;

    /// What the app does with a request, through the C interface: (status, headers, body).
    fn ask(method: &str, url: &str, headers: &str, body: &[u8]) -> (u16, String, Vec<u8>) {
        let c = |text: &str| CString::new(text).unwrap();
        let (method, url, headers) = (c(method), c(url), c(headers));
        unsafe {
            let response = reversal_engine_request(
                method.as_ptr(),
                url.as_ptr(),
                headers.as_ptr(),
                if body.is_empty() {
                    std::ptr::null()
                } else {
                    body.as_ptr()
                },
                body.len(),
            );
            let answer = &*response;
            let got = (
                answer.status,
                CStr::from_ptr(answer.headers).to_str().unwrap().to_string(),
                std::slice::from_raw_parts(answer.body, answer.body_len).to_vec(),
            );
            reversal_response_free(response);
            got
        }
    }

    #[test]
    fn answers_the_page_for_the_open_folder_only() {
        // Without a folder there is nothing to answer with.
        reversal_engine_close();
        assert_eq!(ask("GET", "/api/version", "", b"").0, 503);

        let dir = std::env::temp_dir().join(format!("reversal-ios-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let folder = CString::new(dir.to_str().unwrap()).unwrap();
        assert!(unsafe { reversal_engine_open(folder.as_ptr()) });

        let (status, headers, body) = ask("GET", "/api/version", "", b"");
        assert_eq!(status, 200);
        assert!(headers.contains("Content-Type: application/json\n"));
        assert!(String::from_utf8(body)
            .unwrap()
            .contains(env!("CARGO_PKG_VERSION")));

        // A POST carries its headers and body across.
        let own = format!("Content-Type: application/json\nOrigin: {APP_ORIGIN}\n");
        let vote = br#"{"bases":[[0.5,0.4,0.3]],"readings":[]}"#;
        let (status, _, body) = ask("POST", "/api/roll/vote", &own, vote);
        assert_eq!(status, 200);
        assert!(String::from_utf8(body).unwrap().contains("base_rgb"));
        assert_eq!(ask("POST", "/api/roll/vote", "", vote).0, 403);
        assert_eq!(ask("DELETE", "/api/files", "", b"").0, 404);

        // A folder that can't be opened leaves none open.
        let missing = CString::new("/no/such/folder").unwrap();
        assert!(!unsafe { reversal_engine_open(missing.as_ptr()) });
        assert_eq!(ask("GET", "/api/version", "", b"").0, 503);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn the_app_registers_the_scheme_the_engine_expects() {
        let shell = include_str!("../../../packaging/ios/Reversal/Engine.swift");
        assert!(shell.contains(&format!("\"{APP_ORIGIN}/\"")));
    }

    #[test]
    fn asks_the_engine_which_files_are_frames() {
        let is_scan = |name: &str| {
            let name = CString::new(name).unwrap();
            unsafe { reversal_is_scan(name.as_ptr()) }
        };
        assert!(is_scan("_DSC5384.NEF"));
        assert!(is_scan("frame.dng"));
        assert!(!is_scan("frame-positive.dng"));
        assert!(!is_scan("notes.txt"));
    }
}
