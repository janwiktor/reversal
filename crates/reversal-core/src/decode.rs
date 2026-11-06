//! Raw decoding through the system LibRaw (C API).
//!
//! Output is demosaiced, linear, *unbalanced* camera RGB: the negative is measured as
//! transmittance, so no white balance or camera color matrix is applied.

use std::ffi::{c_char, c_float, c_int, c_uint, c_ushort, c_void, CStr, CString};
use std::path::Path;
use std::sync::Mutex;

use crate::image::Rgb32;

/// `LIBRAW_IMAGE_BITMAP`: pixels, not a JPEG thumbnail.
const IMAGE_BITMAP: c_int = 2;

/// `libraw_processed_image_t`.
#[repr(C)]
struct ProcessedImage {
    kind: c_int,
    height: c_ushort,
    width: c_ushort,
    colors: c_ushort,
    bits: c_ushort,
    data_size: c_uint,
    data: [u8; 1],
}

#[link(name = "raw")]
extern "C" {
    fn libraw_init(flags: c_uint) -> *mut c_void;
    fn libraw_open_file(lr: *mut c_void, path: *const c_char) -> c_int;
    fn libraw_unpack(lr: *mut c_void) -> c_int;
    fn libraw_dcraw_process(lr: *mut c_void) -> c_int;
    fn libraw_dcraw_make_mem_image(lr: *mut c_void, errc: *mut c_int) -> *mut ProcessedImage;
    fn libraw_dcraw_clear_mem(img: *mut ProcessedImage);
    fn libraw_close(lr: *mut c_void);
    fn libraw_strerror(code: c_int) -> *const c_char;
    fn libraw_set_output_color(lr: *mut c_void, value: c_int);
    fn libraw_set_output_bps(lr: *mut c_void, value: c_int);
    fn libraw_set_gamma(lr: *mut c_void, index: c_int, value: c_float);
    fn libraw_set_no_auto_bright(lr: *mut c_void, value: c_int);
    fn libraw_set_user_mul(lr: *mut c_void, index: c_int, value: c_float);
    fn libraw_set_highlight(lr: *mut c_void, value: c_int);
    fn libraw_set_demosaic(lr: *mut c_void, value: c_int);
    fn libraw_set_adjust_maximum_thr(lr: *mut c_void, value: c_float);
}

#[derive(Debug)]
pub struct DecodeError(pub String);

impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "raw decode failed: {}", self.0)
    }
}

impl std::error::Error for DecodeError {}

struct Handle(*mut c_void);

impl Drop for Handle {
    fn drop(&mut self) {
        unsafe { libraw_close(self.0) }
    }
}

fn check(code: c_int, step: &str) -> Result<(), DecodeError> {
    if code == 0 {
        return Ok(());
    }
    let msg = unsafe { CStr::from_ptr(libraw_strerror(code)) }.to_string_lossy();
    Err(DecodeError(format!("{step}: {msg}")))
}

#[cfg(unix)]
fn path_bytes(path: &Path) -> Vec<u8> {
    use std::os::unix::ffi::OsStrExt;
    path.as_os_str().as_bytes().to_vec()
}

#[cfg(not(unix))]
fn path_bytes(path: &Path) -> Vec<u8> {
    path.to_string_lossy().into_owned().into_bytes()
}

/// One decode at a time. We link plain `libraw`, not its thread-safe `libraw_r`, and a build
/// with OpenMP slows to a crawl when several decodes each start a thread for every core.
static DECODING: Mutex<()> = Mutex::new(());

/// Decode a camera raw or DNG to linear camera RGB in [0, 1].
pub fn decode(path: &Path) -> Result<Rgb32, DecodeError> {
    let _one_at_a_time = DECODING.lock().unwrap_or_else(|e| e.into_inner());
    let c_path = CString::new(path_bytes(path))
        .map_err(|_| DecodeError("path contains a NUL byte".into()))?;
    unsafe {
        let lr = libraw_init(0);
        if lr.is_null() {
            return Err(DecodeError("libraw_init failed".into()));
        }
        let handle = Handle(lr);
        libraw_set_output_color(lr, 0); // raw camera space
        libraw_set_output_bps(lr, 16);
        libraw_set_gamma(lr, 0, 1.0);
        libraw_set_gamma(lr, 1, 1.0);
        libraw_set_no_auto_bright(lr, 1);
        libraw_set_highlight(lr, 0); // clip
        libraw_set_demosaic(lr, 3); // AHD
        libraw_set_adjust_maximum_thr(lr, 0.0); // same scale for every frame of a roll
        for i in 0..4 {
            libraw_set_user_mul(lr, i, 1.0);
        }
        check(libraw_open_file(lr, c_path.as_ptr()), "open")?;
        check(libraw_unpack(lr), "unpack")?;
        check(libraw_dcraw_process(lr), "process")?;
        let mut err = 0;
        let img = libraw_dcraw_make_mem_image(lr, &mut err);
        check(err, "make_mem_image")?;
        if img.is_null() {
            return Err(DecodeError("no image returned".into()));
        }
        let (w, h) = ((*img).width as usize, (*img).height as usize);
        let (kind, colors, bits) = ((*img).kind, (*img).colors as usize, (*img).bits);
        let data_size = (*img).data_size as usize;
        // Trust nothing about the buffer before reading it: its size has to be what the header
        // says, and this library build may have been compiled with other options.
        let result = if kind != IMAGE_BITMAP || colors != 3 || bits != 16 {
            Err(DecodeError(format!(
                "unsupported output: kind {kind}, {colors} colors, {bits} bits"
            )))
        } else if w == 0 || h == 0 || data_size != w * h * 3 * 2 {
            Err(DecodeError(format!(
                "image data is {data_size} bytes for {w}x{h} pixels"
            )))
        } else {
            let samples = std::slice::from_raw_parts((*img).data.as_ptr() as *const u16, w * h * 3);
            let data = samples.iter().map(|&v| v as f32 / 65535.0).collect();
            Ok(Rgb32 {
                width: w,
                height: h,
                data,
            })
        };
        libraw_dcraw_clear_mem(img);
        drop(handle);
        result
    }
}
