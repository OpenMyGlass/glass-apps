//! Linux framebuffer back end (`fb` feature), for the device.
//!
//! [`FramebufferDisplay`] presents finished [`Frame`]s (premultiplied RGBA8888) straight to
//! a Linux framebuffer device (`/dev/fb0`). It is the on-device software-renderer sink for
//! "Model B" (plain Linux on Glass): the GLES path (`glass-gfx-gles`) is the normal device
//! renderer, this is the no-GPU fallback that the static-musl debug build uses.
//!
//! Design, in order of what it saves:
//! * **Runtime geometry.** `FBIOGET_VSCREENINFO`/`FBIOGET_FSCREENINFO` give `xres`/`yres`,
//!   `bits_per_pixel`, `line_length` (stride) and the red/green/blue/transp bit positions.
//!   Nothing is hardcoded; the Glass panel reports 640x360, 32bpp, stride 2560, **BGRA**
//!   (red at bit 16, blue at bit 0), and we read that and convert accordingly.
//! * **mmap, no per-frame `write()`.** The framebuffer is mapped once and written in place.
//! * **Dirty rectangles.** The previously presented frame is kept; each `present` compares
//!   it scanline by scanline and, within a changed scanline, narrows to the first..last
//!   changed pixel, converting and copying only that span. A still screen costs one memcmp
//!   per row and no writes; a moving clock digit costs a few short spans.
//! * **Word-wise conversion.** RGBA->fb packing works on 32-bit words. The two common 32bpp
//!   cases are specialised: identity (fb is RGBA) is a `memcpy`; red/blue swap (fb is BGRA,
//!   this panel) is a byte shuffle; anything else uses a general bit-field pack.
//! * **Present only on change.** The launcher's loop already blocks until a frame changes
//!   and only then calls `present`, so there is no polling and no extra thread here.

use crate::{Display, DisplayError, Frame};
use std::fs::OpenOptions;
use std::os::unix::io::{AsRawFd, RawFd};
use std::path::Path;

const FBIOGET_VSCREENINFO: libc::c_ulong = 0x4600;
const FBIOGET_FSCREENINFO: libc::c_ulong = 0x4602;
const FBIOBLANK: libc::c_ulong = 0x4611;
const FB_BLANK_UNBLANK: libc::c_int = 0;

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct FbBitfield {
    offset: u32,
    length: u32,
    msb_right: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct FbVarScreeninfo {
    xres: u32,
    yres: u32,
    xres_virtual: u32,
    yres_virtual: u32,
    xoffset: u32,
    yoffset: u32,
    bits_per_pixel: u32,
    grayscale: u32,
    red: FbBitfield,
    green: FbBitfield,
    blue: FbBitfield,
    transp: FbBitfield,
    nonstd: u32,
    activate: u32,
    height: u32,
    width: u32,
    accel_flags: u32,
    pixclock: u32,
    left_margin: u32,
    right_margin: u32,
    upper_margin: u32,
    lower_margin: u32,
    hsync_len: u32,
    vsync_len: u32,
    sync: u32,
    vmode: u32,
    rotate: u32,
    colorspace: u32,
    reserved: [u32; 4],
}

#[repr(C)]
#[derive(Clone, Copy)]
struct FbFixScreeninfo {
    id: [u8; 16],
    smem_start: libc::c_ulong,
    smem_len: u32,
    type_: u32,
    type_aux: u32,
    visual: u32,
    xpanstep: u16,
    ypanstep: u16,
    ywrapstep: u16,
    line_length: u32,
    mmio_start: libc::c_ulong,
    mmio_len: u32,
    accel: u32,
    capabilities: u16,
    reserved: [u16; 2],
}

impl Default for FbFixScreeninfo {
    fn default() -> Self {
        // All-zero is a valid "unset" fixed-info; the ioctl fills it.
        unsafe { std::mem::zeroed() }
    }
}

/// How a frame's RGBA pixel becomes the framebuffer's pixel.
#[derive(Clone, Copy)]
enum Convert {
    /// fb is RGBA8888 (offsets 0/8/16/24): a straight copy.
    Identity,
    /// fb is BGRA8888 (offsets 16/8/0/24): swap red and blue.
    SwapRb,
    /// Any other layout: pack each channel into its bit-field. `bytes` is bytes per pixel.
    General {
        r: (u32, u32),
        g: (u32, u32),
        b: (u32, u32),
        a: (u32, u32),
        bytes: usize,
    },
}

fn field(bf: &FbBitfield) -> (u32, u32) {
    (bf.offset, bf.length)
}

/// A Linux framebuffer that shows frames of its own `xres`x`yres` size.
pub struct FramebufferDisplay {
    _file: std::fs::File, // keeps the fd (and the mapping) alive
    fd: RawFd,
    map: *mut u8,
    map_len: usize,
    xres: u32,
    yres: u32,
    line_length: usize,
    bytes_pp: usize,
    convert: Convert,
    /// The last presented frame's RGBA bytes (`yres * xres * 4`), for dirty diffing.
    last: Vec<u8>,
    first: bool,
    presents: u64,
}

impl FramebufferDisplay {
    /// Opens a framebuffer device (for example `/dev/fb0`), reads its geometry and maps it.
    ///
    /// Also does a best-effort panel bring-up (unblank + brightness); see
    /// [`FramebufferDisplay::bring_up_panel`]. Errors from that are only logged, never fatal,
    /// so this works on any Linux fb, not just Glass.
    pub fn open(path: impl AsRef<Path>) -> Result<FramebufferDisplay, DisplayError> {
        let path = path.as_ref();
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
            .map_err(|e| DisplayError::Io(format!("open {}: {e}", path.display())))?;
        let fd = file.as_raw_fd();

        let mut vinfo = FbVarScreeninfo::default();
        let mut finfo = FbFixScreeninfo::default();
        unsafe {
            if libc::ioctl(fd, FBIOGET_VSCREENINFO as _, &mut vinfo) < 0 {
                return Err(DisplayError::Io(format!(
                    "FBIOGET_VSCREENINFO: {}",
                    std::io::Error::last_os_error()
                )));
            }
            if libc::ioctl(fd, FBIOGET_FSCREENINFO as _, &mut finfo) < 0 {
                return Err(DisplayError::Io(format!(
                    "FBIOGET_FSCREENINFO: {}",
                    std::io::Error::last_os_error()
                )));
            }
        }

        let bpp = vinfo.bits_per_pixel;
        if bpp == 0 || bpp % 8 != 0 {
            return Err(DisplayError::Io(format!("unsupported bits_per_pixel {bpp}")));
        }
        let bytes_pp = (bpp / 8) as usize;
        let line_length = if finfo.line_length != 0 {
            finfo.line_length as usize
        } else {
            vinfo.xres as usize * bytes_pp
        };
        // Map the whole visible+virtual area (what the kernel exposes for this device).
        let map_len = {
            let by_stride = line_length * vinfo.yres_virtual.max(vinfo.yres) as usize;
            if finfo.smem_len as usize >= by_stride && finfo.smem_len != 0 {
                finfo.smem_len as usize
            } else {
                by_stride
            }
        };
        if map_len < line_length * vinfo.yres as usize {
            return Err(DisplayError::Io("framebuffer smaller than its mode".into()));
        }

        let map = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                map_len,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_SHARED,
                fd,
                0,
            )
        };
        if map == libc::MAP_FAILED {
            return Err(DisplayError::Io(format!(
                "mmap {}: {}",
                path.display(),
                std::io::Error::last_os_error()
            )));
        }

        let convert = choose_convert(&vinfo, bytes_pp);

        eprintln!(
            "glass-launcher: fb: {} {}x{} {}bpp stride={} fmt=R{}/{} G{}/{} B{}/{} A{}/{} -> {}",
            path.display(),
            vinfo.xres,
            vinfo.yres,
            bpp,
            line_length,
            vinfo.red.offset,
            vinfo.red.length,
            vinfo.green.offset,
            vinfo.green.length,
            vinfo.blue.offset,
            vinfo.blue.length,
            vinfo.transp.offset,
            vinfo.transp.length,
            convert.name(),
        );

        let mut disp = FramebufferDisplay {
            _file: file,
            fd,
            map: map as *mut u8,
            map_len,
            xres: vinfo.xres,
            yres: vinfo.yres,
            line_length,
            bytes_pp,
            convert,
            last: vec![0u8; vinfo.xres as usize * vinfo.yres as usize * 4],
            first: true,
            presents: 0,
        };
        // Best-effort: make sure the panel is lit (unblank + brightness). Never fatal.
        disp.bring_up_panel(Some(200));
        Ok(disp)
    }

    /// The device panel's "mini lights HAL": unblank the fb and, if a Glass notle-dpi panel
    /// is present, raise its backlight brightness. Pure best-effort; failures are logged.
    ///
    /// `brightness` of `None` leaves the brightness alone (only unblank).
    pub fn bring_up_panel(&mut self, brightness: Option<u32>) {
        // Unblank via ioctl (works on any fbdev) ...
        unsafe {
            libc::ioctl(self.fd, FBIOBLANK as _, FB_BLANK_UNBLANK);
        }
        // ... and via sysfs (some stacks only honour the sysfs attribute).
        let _ = std::fs::write("/sys/class/graphics/fb0/blank", b"0\n");

        if let Some(b) = brightness {
            let val = b.to_string();
            // The Glass panel's brightness lives under the DSS manager it hangs off:
            // /sys/devices/platform/omapdss/manager*/panel-notle-dpi/brightness. Paths vary,
            // so find it; also try the generic backlight class.
            let mut wrote = false;
            if let Ok(mgrs) = std::fs::read_dir("/sys/devices/platform/omapdss") {
                for m in mgrs.flatten() {
                    let p = m.path().join("panel-notle-dpi/brightness");
                    if p.exists() && std::fs::write(&p, val.as_bytes()).is_ok() {
                        wrote = true;
                    }
                }
            }
            if let Ok(bls) = std::fs::read_dir("/sys/class/backlight") {
                for bl in bls.flatten() {
                    let _ = std::fs::write(bl.path().join("brightness"), val.as_bytes());
                }
            }
            if !wrote {
                eprintln!("glass-launcher: fb: no panel-notle-dpi brightness node (ok off-device)");
            }
        }
    }

}

impl Convert {
    fn name(&self) -> &'static str {
        match self {
            Convert::Identity => "memcpy(RGBA)",
            Convert::SwapRb => "swap-R/B(BGRA)",
            Convert::General { .. } => "bitfield",
        }
    }

    /// Converts `src` (RGBA, 4 bytes/pixel) into `dst` (the fb's layout). `src.len()` is a
    /// multiple of 4; `dst.len()` is `src.len()/4 * bytes_pp`.
    #[inline]
    fn span(&self, src: &[u8], dst: &mut [u8]) {
        match *self {
            Convert::Identity => dst.copy_from_slice(src),
            Convert::SwapRb => {
                for (s, d) in src.chunks_exact(4).zip(dst.chunks_exact_mut(4)) {
                    d[0] = s[2]; // B
                    d[1] = s[1]; // G
                    d[2] = s[0]; // R
                    d[3] = s[3]; // A
                }
            }
            Convert::General { r, g, b, a, bytes } => {
                for (s, d) in src.chunks_exact(4).zip(dst.chunks_exact_mut(bytes)) {
                    let pack = |val: u8, (off, len): (u32, u32)| -> u32 {
                        if len == 0 {
                            return 0;
                        }
                        // Scale the 8-bit channel down to the field width, then position it.
                        let v = (val as u32) >> (8 - len.min(8));
                        (v & ((1u32 << len) - 1)) << off
                    };
                    let word = pack(s[0], r) | pack(s[1], g) | pack(s[2], b) | pack(s[3], a);
                    let wb = word.to_le_bytes();
                    d.copy_from_slice(&wb[..bytes]);
                }
            }
        }
    }
}

fn choose_convert(vinfo: &FbVarScreeninfo, bytes_pp: usize) -> Convert {
    let (r, g, b, a) = (
        field(&vinfo.red),
        field(&vinfo.green),
        field(&vinfo.blue),
        field(&vinfo.transp),
    );
    if bytes_pp == 4 {
        // Channel lengths are 8 for all the 32bpp cases we specialise.
        if r == (0, 8) && g == (8, 8) && b == (16, 8) {
            return Convert::Identity;
        }
        if r == (16, 8) && g == (8, 8) && b == (0, 8) {
            return Convert::SwapRb;
        }
    }
    Convert::General {
        r,
        g,
        b,
        a,
        bytes: bytes_pp,
    }
}

/// First and last changed pixel index in a row (`[lo, hi)`), comparing 4-byte pixels.
/// Only called when the rows are known to differ, so `lo < hi` always holds.
#[inline]
fn changed_span(new: &[u8], old: &[u8]) -> (usize, usize) {
    let n = new.len() / 4;
    let mut lo = 0usize;
    while lo < n && new[lo * 4..lo * 4 + 4] == old[lo * 4..lo * 4 + 4] {
        lo += 1;
    }
    let mut hi = n;
    while hi > lo && new[(hi - 1) * 4..hi * 4] == old[(hi - 1) * 4..hi * 4] {
        hi -= 1;
    }
    (lo, hi)
}

impl Display for FramebufferDisplay {
    fn size(&self) -> (u32, u32) {
        (self.xres, self.yres)
    }

    fn present(&mut self, frame: &Frame) -> Result<(), DisplayError> {
        if (frame.width(), frame.height()) != (self.xres, self.yres) {
            return Err(DisplayError::SizeMismatch {
                expected: (self.xres, self.yres),
                got: (frame.width(), frame.height()),
            });
        }
        let w = self.xres as usize;
        let h = self.yres as usize;
        let src_stride = w * 4;
        let src = frame.data();

        // Copy the fields we need before taking the long-lived &mut to the mapping.
        let (convert, line_length, bytes_pp, first) =
            (self.convert, self.line_length, self.bytes_pp, self.first);

        let mut changed_rows = 0u32;
        let mut changed_px = 0u64;

        // Split borrows: `last` and the mapping are disjoint.
        let last = std::mem::take(&mut self.last);
        let mut last = last; // owned; put back at the end
        {
            let fb = unsafe { std::slice::from_raw_parts_mut(self.map, self.map_len) };
            for y in 0..h {
                let so = y * src_stride;
                let srow = &src[so..so + src_stride];
                let lrow = &mut last[so..so + src_stride];
                if !first && srow == lrow {
                    continue;
                }
                let (lo, hi) = if first { (0, w) } else { changed_span(srow, lrow) };
                if lo >= hi {
                    continue;
                }
                let fb_off = y * line_length + lo * bytes_pp;
                let fb_span = &mut fb[fb_off..fb_off + (hi - lo) * bytes_pp];
                convert.span(&srow[lo * 4..hi * 4], fb_span);
                lrow[lo * 4..hi * 4].copy_from_slice(&srow[lo * 4..hi * 4]);
                changed_rows += 1;
                changed_px += (hi - lo) as u64;
            }
        }
        self.last = last;
        self.first = false;
        self.presents += 1;

        if self.presents == 1 || self.presents % 120 == 0 {
            eprintln!(
                "glass-launcher: fb: present #{} ({} rows, {} px changed)",
                self.presents, changed_rows, changed_px
            );
        }
        Ok(())
    }
}

impl Drop for FramebufferDisplay {
    fn drop(&mut self) {
        if !self.map.is_null() {
            unsafe {
                libc::munmap(self.map as *mut libc::c_void, self.map_len);
            }
        }
    }
}
