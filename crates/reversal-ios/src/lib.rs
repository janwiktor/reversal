// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! C interface for the iOS app. An iOS app can't start the `reversal` binary, so it runs the same
//! web UI server on a thread of its own process. See `packaging/ios/Engine.h`.

use std::ffi::{c_char, CStr};
use std::path::PathBuf;
use std::sync::Mutex;

use reversal_cli::batch::lock;
use reversal_cli::folder::is_scan_name;
use reversal_cli::ui::InProcess;

/// One folder at a time, as with the Mac app's engine process.
static ENGINE: Mutex<Option<InProcess>> = Mutex::new(None);

/// Serve `folder` in place of what was served, and return the port, or 0 if that failed.
///
/// # Safety
/// `folder` must be a NUL-terminated string.
#[no_mangle]
pub unsafe extern "C" fn reversal_engine_start(folder: *const c_char) -> u16 {
    let mut engine = lock(&ENGINE);
    *engine = None;
    let Ok(folder) = CStr::from_ptr(folder).to_str() else {
        return 0;
    };
    match InProcess::start(PathBuf::from(folder)) {
        Ok(started) => engine.insert(started).port,
        Err(e) => {
            eprintln!("could not start the engine: {e}");
            0
        }
    }
}

#[no_mangle]
pub extern "C" fn reversal_engine_stop() {
    *lock(&ENGINE) = None;
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
    use std::ffi::CString;
    use std::io::{Read, Write};
    use std::net::TcpStream;

    fn answers(port: u16) -> bool {
        let request = format!(
            "GET /api/version HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"
        );
        let mut reply = String::new();
        TcpStream::connect(("127.0.0.1", port))
            .and_then(|mut stream| {
                stream.write_all(request.as_bytes())?;
                stream.read_to_string(&mut reply)
            })
            .is_ok()
            && reply.contains(env!("CARGO_PKG_VERSION"))
    }

    /// A stopped server lets go of its port on its own thread, a moment later.
    fn stops_answering(port: u16) -> bool {
        (0..200).any(|_| {
            std::thread::sleep(std::time::Duration::from_millis(10));
            !answers(port)
        })
    }

    #[test]
    fn serves_a_folder_and_lets_go_of_it_for_the_next() {
        let dir = std::env::temp_dir();
        let folder = CString::new(dir.to_str().unwrap()).unwrap();
        let first = unsafe { reversal_engine_start(folder.as_ptr()) };
        assert_ne!(first, 0);
        assert!(answers(first));
        let second = unsafe { reversal_engine_start(folder.as_ptr()) };
        assert_ne!(second, 0);
        assert!(answers(second));
        assert!(stops_answering(first));
        reversal_engine_stop();
        assert!(stops_answering(second));
        let missing = CString::new("/no/such/folder").unwrap();
        assert_eq!(unsafe { reversal_engine_start(missing.as_ptr()) }, 0);
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
