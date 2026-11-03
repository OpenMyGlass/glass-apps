//! On-device UI sound for Model B: a tiny, self-contained ALSA output path.
//!
//! No `libasound`, no `tinyalsa`, no external tools (the Alpine image ships none): this
//! talks to the kernel's sound devices directly with the documented `SNDRV_*` ioctls on
//! `/dev/snd/controlC0` (mixer) and `/dev/snd/pcmC0D0p` (playback), exactly as a small
//! tinyalsa would. It keeps the static musl binary small and dependency-free.
//!
//! The important output is the Glass **bone-conduction transducer**, driven by the
//! TWL6040 **Earphone (EP)** output. Before playback we set the same mixer controls the
//! stock Android audio HAL sets for the EP path (card 0 "Notle"), from
//! `android_device_glass_glass-1` `docs/specs/audio.md` section 4. A USB earbud/headset
//! is handled as a secondary path (Headset L/R on, EP off).
//!
//! Playback is card 0, device 0 (OMAP ABE Multimedia front-end), S16_LE, 48 kHz, stereo,
//! plain blocking `writei` (no mmap) — the Model-B path the spec calls out as the one to
//! confirm. UI sounds are synthesised in-code (see [`synth`]); nothing is read from disk.
//!
//! Only built with the `sound` feature on Linux. Errors are reported (for the init's
//! `/dev/kmsg` log) but never fatal: feedback is best-effort.

#![allow(clippy::unusual_byte_groupings)]

use crate::feedback::{Cue, Feedback};
use std::ffi::CString;
use std::fs;
use std::io::{self, Error, ErrorKind};
use std::os::unix::io::RawFd;
use std::sync::mpsc;
use std::thread;

// ---- kernel ABI (sound/asound.h), 32-bit ARM (OMAP4430, kernel 3.4.x) --------------

const CARD: u32 = 0;
const DEVICE: u32 = 0;

const RATE: u32 = 48_000;
const CHANNELS: u32 = 2;

// struct sizes, 32-bit ARM EABI (used in the ioctl numbers, so they must be exact).
const SIZEOF_HW_PARAMS: usize = 604;
const SIZEOF_SW_PARAMS: usize = 104;
const SIZEOF_CTL_ELEM_VALUE: usize = 712;

// _IOC(dir,type,nr,size) = (dir<<30)|(size<<16)|(type<<8)|nr ; READ|WRITE dir = 3.
const fn ioc(dir: u32, ty: u32, nr: u32, size: usize) -> libc::c_ulong {
    ((dir << 30) | ((size as u32) << 16) | (ty << 8) | nr) as libc::c_ulong
}
const IOC_NONE: u32 = 0;
const IOC_WRITE: u32 = 1;
const IOC_READ: u32 = 2;
const A: u32 = b'A' as u32;
const U: u32 = b'U' as u32;

const SNDRV_PCM_IOCTL_HW_PARAMS: libc::c_ulong =
    ioc(IOC_READ | IOC_WRITE, A, 0x11, SIZEOF_HW_PARAMS);
const SNDRV_PCM_IOCTL_SW_PARAMS: libc::c_ulong =
    ioc(IOC_READ | IOC_WRITE, A, 0x13, SIZEOF_SW_PARAMS);
const SNDRV_PCM_IOCTL_PREPARE: libc::c_ulong = ioc(IOC_NONE, A, 0x40, 0);
const SNDRV_PCM_IOCTL_START: libc::c_ulong = ioc(IOC_NONE, A, 0x42, 0);
const SNDRV_PCM_IOCTL_DRAIN: libc::c_ulong = ioc(IOC_NONE, A, 0x44, 0);
const SNDRV_PCM_IOCTL_DROP: libc::c_ulong = ioc(IOC_NONE, A, 0x43, 0);
const SNDRV_PCM_IOCTL_WRITEI_FRAMES: libc::c_ulong = ioc(IOC_WRITE, A, 0x50, 12);
const SNDRV_PCM_IOCTL_HWSYNC: libc::c_ulong = ioc(IOC_NONE, A, 0x22, 0);
const SIZEOF_SYNC_PTR: usize = 132; // flags(4) + status union(64) + control union(64)
const SNDRV_PCM_IOCTL_SYNC_PTR: libc::c_ulong = ioc(IOC_READ | IOC_WRITE, A, 0x23, SIZEOF_SYNC_PTR);
const SNDRV_CTL_IOCTL_ELEM_WRITE: libc::c_ulong =
    ioc(IOC_READ | IOC_WRITE, U, 0x13, SIZEOF_CTL_ELEM_VALUE);
const SNDRV_CTL_IOCTL_ELEM_READ: libc::c_ulong =
    ioc(IOC_READ | IOC_WRITE, U, 0x12, SIZEOF_CTL_ELEM_VALUE);
const SIZEOF_CTL_ELEM_LIST: usize = 72;
const SNDRV_CTL_IOCTL_ELEM_LIST: libc::c_ulong =
    ioc(IOC_READ | IOC_WRITE, U, 0x10, SIZEOF_CTL_ELEM_LIST);

// hw_params layout (byte offsets inside the 604-byte struct).
const HWP_MASK0: usize = 4; // masks[0] (ACCESS); each snd_mask is 32 bytes.
const HWP_INTERVAL0: usize = 260; // intervals[0] (SAMPLE_BITS); each snd_interval is 12 bytes.
const HWP_RMASK: usize = 512;
const HWP_INFO: usize = 520;

// parameter ids
const P_ACCESS: usize = 0;
const P_FORMAT: usize = 1;
const P_CHANNELS: usize = 10;
const P_RATE: usize = 11;
const P_PERIOD_SIZE: usize = 13;
const P_PERIODS: usize = 15;
const P_BUFFER_SIZE: usize = 17;
const FIRST_INTERVAL: usize = 8;

const SNDRV_PCM_ACCESS_MMAP_INTERLEAVED: u32 = 0;
const SNDRV_PCM_ACCESS_RW_INTERLEAVED: u32 = 3;
const SNDRV_PCM_FORMAT_S16_LE: u32 = 2;
const SNDRV_PCM_FORMAT_S32_LE: u32 = 10;
// hw_params.flags bit0 = NO_PERIOD_WAKEUP (what tinyalsa's PCM_NOIRQ sets).
const SNDRV_PCM_HW_PARAMS_NO_PERIOD_WAKEUP: u32 = 1 << 0;
// snd_pcm_sync_ptr flags
const SNDRV_PCM_SYNC_PTR_HWSYNC: u32 = 1 << 0;
// byte offsets inside snd_pcm_sync_ptr (32-bit ARM)
const SP_FLAGS: usize = 0;
const SP_STATUS_HWPTR: usize = 4 + 8; // status union @4; hw_ptr @ status+8
const SP_CTRL_APPLPTR: usize = 4 + 64; // control union @68; appl_ptr @ +0
const SP_CTRL_AVAILMIN: usize = 4 + 64 + 4; // avail_min @ +4

const SNDRV_CTL_ELEM_IFACE_MIXER: u32 = 2;

// ---- small ioctl / fd helpers ------------------------------------------------------

struct Fd(RawFd);
impl Drop for Fd {
    fn drop(&mut self) {
        unsafe { libc::close(self.0) };
    }
}

fn errno() -> i32 {
    Error::last_os_error().raw_os_error().unwrap_or(0)
}

/// " (errno N)" when the error carries an OS errno, else "".
fn errno_suffix(e: &Error) -> String {
    match e.raw_os_error() {
        Some(n) => format!(" (errno {n})"),
        None => String::new(),
    }
}

/// Opens `/dev/snd/<name>`, creating the node from `/sys/class/sound/<name>/dev` if the
/// device tmpfs has no entry yet (mdev may not have placed it under `snd/`).
fn open_snd(name: &str) -> io::Result<Fd> {
    let path = format!("/dev/snd/{name}");
    if let Some(fd) = try_open(&path) {
        return Ok(fd);
    }
    // Create /dev/snd and the node, then retry.
    let _ = fs::create_dir_all("/dev/snd");
    let dev = fs::read_to_string(format!("/sys/class/sound/{name}/dev"))
        .map_err(|e| Error::new(e.kind(), format!("read dev of {name}: {e}")))?;
    let dev = dev.trim();
    let (maj, min) = dev
        .split_once(':')
        .and_then(|(a, b)| Some((a.parse::<u32>().ok()?, b.parse::<u32>().ok()?)))
        .ok_or_else(|| {
            Error::new(
                ErrorKind::InvalidData,
                format!("bad dev {dev:?} for {name}"),
            )
        })?;
    let cpath = CString::new(path.clone()).unwrap();
    let rc = unsafe {
        libc::mknod(
            cpath.as_ptr(),
            libc::S_IFCHR | 0o600,
            libc::makedev(maj, min),
        )
    };
    if rc != 0 && errno() != libc::EEXIST {
        return Err(Error::new(
            Error::last_os_error().kind(),
            format!("mknod {path} c {maj} {min}: {}", Error::last_os_error()),
        ));
    }
    try_open(&path).ok_or_else(|| Error::other(format!("open {path}: {}", Error::last_os_error())))
}

fn try_open(path: &str) -> Option<Fd> {
    let c = CString::new(path).ok()?;
    let fd = unsafe { libc::open(c.as_ptr(), libc::O_RDWR | libc::O_CLOEXEC) };
    if fd < 0 {
        None
    } else {
        Some(Fd(fd))
    }
}

fn ioctl_ptr(fd: RawFd, req: libc::c_ulong, arg: *mut libc::c_void) -> io::Result<()> {
    let rc = unsafe { libc::ioctl(fd, req as _, arg) };
    if rc < 0 {
        Err(Error::last_os_error())
    } else {
        Ok(())
    }
}

// ---- mixer (control device) --------------------------------------------------------

/// The TWL6040/ABE mixer on card 0.
pub struct Mixer {
    fd: Fd,
}

impl Mixer {
    pub fn open() -> io::Result<Mixer> {
        Ok(Mixer {
            fd: open_snd(&format!("controlC{CARD}"))?,
        })
    }

    /// Writes an integer/boolean control by name (numid 0 → kernel resolves by name),
    /// setting the first two values (covers mono and stereo controls).
    pub fn set(&self, name: &str, value: i32) -> io::Result<()> {
        if name.len() >= 44 {
            return Err(Error::new(ErrorKind::InvalidInput, "control name too long"));
        }
        let mut buf = [0u8; SIZEOF_CTL_ELEM_VALUE];
        // id.numid = 0 (offset 0); id.iface = MIXER (offset 4); id.name (offset 16).
        put_u32(&mut buf, 4, SNDRV_CTL_ELEM_IFACE_MIXER);
        buf[16..16 + name.len()].copy_from_slice(name.as_bytes());
        // value.integer.value[0..2] at offset 72 (union is 8-byte aligned after id+indirect).
        put_i32(&mut buf, 72, value);
        put_i32(&mut buf, 76, value);
        ioctl_ptr(
            self.fd.0,
            SNDRV_CTL_IOCTL_ELEM_WRITE,
            buf.as_mut_ptr() as *mut _,
        )
        .map_err(|e| Error::new(e.kind(), format!("set {name:?}={value}: {e}")))
    }

    /// Reads an integer/boolean control by name; returns its first two values.
    pub fn get(&self, name: &str) -> io::Result<[i32; 2]> {
        if name.len() >= 44 {
            return Err(Error::new(ErrorKind::InvalidInput, "control name too long"));
        }
        let mut buf = [0u8; SIZEOF_CTL_ELEM_VALUE];
        put_u32(&mut buf, 4, SNDRV_CTL_ELEM_IFACE_MIXER);
        buf[16..16 + name.len()].copy_from_slice(name.as_bytes());
        ioctl_ptr(
            self.fd.0,
            SNDRV_CTL_IOCTL_ELEM_READ,
            buf.as_mut_ptr() as *mut _,
        )
        .map_err(|e| Error::new(e.kind(), format!("get {name:?}: {e}")))?;
        Ok([get_i32(&buf, 72), get_i32(&buf, 76)])
    }

    /// Lists every control on the card: (numid, iface, name). For diagnosis — so we see
    /// the exact control names the driver exposes.
    pub fn list(&self) -> io::Result<Vec<(u32, u32, String)>> {
        // First pass: ask for the count (space = 0).
        let mut hdr = [0u8; SIZEOF_CTL_ELEM_LIST];
        ioctl_ptr(
            self.fd.0,
            SNDRV_CTL_IOCTL_ELEM_LIST,
            hdr.as_mut_ptr() as *mut _,
        )
        .map_err(|e| Error::new(e.kind(), format!("ELEM_LIST count: {e}")))?;
        let count = get_u32(&hdr, 12) as usize; // field `count`
        if count == 0 {
            return Ok(Vec::new());
        }
        // Second pass: give it an array of `count` snd_ctl_elem_id (64 bytes each).
        let mut ids = vec![0u8; count * 64];
        put_u32(&mut hdr, 0, 0); // offset = 0
        put_u32(&mut hdr, 4, count as u32); // space = count
        put_u32(&mut hdr, 16, ids.as_mut_ptr() as u32); // pids pointer (32-bit)
        ioctl_ptr(
            self.fd.0,
            SNDRV_CTL_IOCTL_ELEM_LIST,
            hdr.as_mut_ptr() as *mut _,
        )
        .map_err(|e| Error::new(e.kind(), format!("ELEM_LIST fetch: {e}")))?;
        let used = get_u32(&hdr, 8) as usize; // field `used`
        let mut out = Vec::with_capacity(used);
        for i in 0..used {
            let base = i * 64;
            let numid = get_u32(&ids, base);
            let iface = get_u32(&ids, base + 4);
            let name_bytes = &ids[base + 16..base + 60];
            let end = name_bytes.iter().position(|&b| b == 0).unwrap_or(44);
            let name = String::from_utf8_lossy(&name_bytes[..end]).into_owned();
            out.push((numid, iface, name));
        }
        Ok(out)
    }
}

/// The mixer controls for a route, in the order the stock HAL applies them.
/// `headset` selects the USB earbud path (Headset L/R) instead of the bone-conduction EP.
///
/// Returns every control it tried with the result, so the caller can log each.
pub fn route_controls(headset: bool) -> Vec<(&'static str, i32)> {
    // Common DL1 -> McPDM -> TWL6040 path (ABE), 0 dB.
    let mut v = vec![
        ("DL1 Mixer Multimedia", 1),
        ("DL1 Media Playback Volume", 120), // ABE gain, 0 dB
        ("DL1 PDM Switch", 1),
    ];
    if headset {
        // USB earbud: HSOL/HSOR on, EP off.
        v.push(("Earphone Playback Switch", 0));
        v.push(("Headset Left Playback", 1));
        v.push(("Headset Right Playback", 1));
        v.push(("Headset Playback Volume", 12)); // TWL6040 HS gain, -6 dB
    } else {
        // Bone-conduction transducer on the TWL6040 earpiece (EP) output.
        v.push(("Headset Left Playback", 0));
        v.push(("Headset Right Playback", 0));
        v.push(("Earphone Playback Switch", 1));
        // TWL6040 EP gain 12 = 0 dB: the stock HAL default and the safe level WITHOUT the
        // stock BCT limiter (libglassalsa). Running at the +6 dB cap (15) with a loud,
        // unlimited synth clips the earpiece amp — gross distortion ("pessimo"). The
        // audibility sweep (--audio-sweep) explores higher volumes to find a clean level.
        v.push(("Earphone Playback Volume", 12));
    }
    v
}

/// The full cold-start **priming** control set — the "V5" recipe confirmed working on the
/// real Glass. On a cold boot, neither route-first alone nor route-first + a minimal PLL
/// tweak made the OMAP4 ABE → McPDM → TWL6040 back-end clock; this full set did. Once it
/// has clocked the codec **once** (with [`Pcm::play_explicit`], an explicit START), the
/// path stays primed and every later open drains normally, in any order.
///
/// It is: the codec power/PLL in the **stock** modes (Low-Power PLL + Low-Power codec,
/// High-Performance headset DAC), the stock ABE DL1 mixer settings (spec 4.2), and the
/// output [`route_controls`].
/// Applied **before** opening the front-end PCM (so DPCM links the McPDM back-end at open).
///
/// | Control | Value | Why |
/// |---|---|---|
/// | `PLL Selection` | 0 = Low-Power (LPPLL) | stock; the HPPLL/MCLK path is silent on Glass |
/// | `TWL6040 Power Mode` | 0 = Low-Power | stock codec analog mode |
/// | `Headset Power Mode` | 1 = High-Performance | stock; HSDAC (feeds the Earphone) |
/// | `DL1 Mono Mixer` | 0 | off (stock) |
/// | `Sidetone Mixer Playback` | 1 | stock DL1 path |
/// | `DL1 Mixer Voice` | 0 | only Multimedia feeds DL1 |
/// | …then [`route_controls`] | | DL1 Mixer Multimedia / Media Vol / PDM Switch + EP (or Headset) output |
pub fn prime_controls(headset: bool) -> Vec<(&'static str, i32)> {
    let mut v = vec![
        // Stock power modes, captured live from the audible stock HAL on 2026-11-03 (gold
        // reference, same kernel, same pcm params): the codec clocks from the Low-Power PLL
        // (32 kHz). High-Performance (HPPLL, from the 38.4 MHz MCLK) left every register,
        // DAPM widget and ABE port looking right but the earpiece SILENT: the owner heard
        // the Low-Power variant of --audio-analog only (one of three patterned tones).
        ("PLL Selection", 0), // Low-Power PLL (stock; HPPLL = silent on Glass)
        ("TWL6040 Power Mode", 0), // Low-Power (stock)
        ("Headset Power Mode", 1), // High-Performance (stock: HSDAC feeds the Earphone)
        ("DL1 Mono Mixer", 0),
        ("Sidetone Mixer Playback", 1),
        ("DL1 Mixer Voice", 0),
    ];
    v.extend(route_controls(headset));
    v
}

/// Reads back the full earpiece/codec path state for diagnosis (logged after priming): if
/// a tone is still inaudible at max volume this shows the actual analog state to check next
/// (HP power modes, PDM routing, DL1/EP gains, headset-vs-EP selection).
pub fn ep_state_report(mixer: &Mixer) -> Vec<String> {
    const EP_CONTROLS: &[&str] = &[
        "DL1 Mixer Multimedia",
        "DL1 Media Playback Volume",
        "DL1 PDM Switch",
        "Earphone Playback Switch",
        "Earphone Playback Volume",
        "Headset Left Playback",
        "Headset Right Playback",
        "Headset Playback Volume",
        "Headset Power Mode",
        "TWL6040 Power Mode",
        "PLL Selection",
        "Sidetone Mixer Playback",
        "DL1 Mono Mixer",
    ];
    let mut out = vec!["EP mixer read-back:".to_string()];
    for name in EP_CONTROLS {
        match mixer.get(name) {
            Ok(v) => out.push(format!("  {name} = {v:?}")),
            Err(e) => out.push(format!("  {name}: read FAIL: {e}")),
        }
    }
    out
}

/// Applies [`prime_controls`] on the mixer, returning the per-control results for logging.
pub fn set_prime(mixer: &Mixer, headset: bool) -> Vec<(&'static str, i32, io::Result<()>)> {
    prime_controls(headset)
        .into_iter()
        .map(|(name, val)| {
            let r = mixer.set(name, val);
            (name, val, r)
        })
        .collect()
}

/// True if a USB earbud/headset is plugged into the Glass USB connector (the HAL reads
/// this sysfs attribute, which measures the ID-pin resistor).
pub fn headset_present() -> bool {
    match fs::read_to_string("/sys/devices/platform/usb_mux.0/headset") {
        Ok(s) => {
            let s = s.trim();
            s.starts_with("Mono") || s.starts_with("Stereo")
        }
        Err(_) => false,
    }
}

/// Sets the output route on the mixer. Returns the per-control results for logging.
pub fn set_route(mixer: &Mixer, headset: bool) -> Vec<(&'static str, i32, io::Result<()>)> {
    route_controls(headset)
        .into_iter()
        .map(|(name, val)| {
            let r = mixer.set(name, val);
            (name, val, r)
        })
        .collect()
}

// ---- PCM playback ------------------------------------------------------------------

#[repr(C)]
struct Xferi {
    result: libc::c_long,
    buf: *const libc::c_void,
    frames: libc::c_ulong,
}

pub struct Pcm {
    fd: Fd,
    pub period_size: u32,
    pub buffer_size: u32,
}

impl Pcm {
    /// Opens card 0 device 0 for playback, S16_LE / 48 kHz / stereo, and configures it.
    pub fn open() -> io::Result<Pcm> {
        Self::open_inner(&mut |_| {})
    }

    /// As [`Pcm::open`], but reports each hw_params attempt (which period/count set
    /// succeeded or the errno/stage of each failure) through `emit` — for the diagnostic.
    pub fn open_diag(emit: &mut dyn FnMut(&str)) -> io::Result<Pcm> {
        Self::open_inner(emit)
    }

    fn open_inner(emit: &mut dyn FnMut(&str)) -> io::Result<Pcm> {
        let fd = match open_snd(&format!("pcmC{CARD}D{DEVICE}p")) {
            Ok(fd) => fd,
            Err(e) => {
                emit(&format!("pcmC{CARD}D{DEVICE}p open: FAIL: {e}"));
                return Err(e);
            }
        };
        // Try the ABE period from the spec first, then smaller/unconstrained fall-backs.
        let attempts: [(u32, u32); 4] = [(14784, 2), (1920, 4), (1024, 4), (0, 0)];
        let mut last = Error::other("no attempt");
        for (ps, pc) in attempts {
            match set_hw_params(&fd, ps, pc) {
                Ok((period_size, buffer_size)) => {
                    set_sw_params(&fd, period_size, buffer_size)?;
                    ioctl_ptr(fd.0, SNDRV_PCM_IOCTL_PREPARE, std::ptr::null_mut())
                        .map_err(|e| Error::new(e.kind(), format!("PREPARE: {e}")))?;
                    emit(&format!(
                        "HW_PARAMS ok: requested period {ps}/{pc} -> period {period_size} buffer {buffer_size} frames @ {RATE} Hz S16_LE stereo"
                    ));
                    return Ok(Pcm {
                        fd,
                        period_size,
                        buffer_size,
                    });
                }
                Err(e) => {
                    emit(&format!(
                        "HW_PARAMS period {ps}/{pc}: FAIL: {e}{}",
                        errno_suffix(&e)
                    ));
                    last = Error::new(e.kind(), format!("HW_PARAMS (period {ps}/{pc}): {e}"));
                }
            }
        }
        Err(last)
    }

    /// Writes interleaved S16 stereo frames (blocking), then drains so the sound plays
    /// out fully. Recovers once from an underrun.
    pub fn play(&self, samples: &[i16]) -> io::Result<()> {
        let _ = ioctl_ptr(self.fd.0, SNDRV_PCM_IOCTL_PREPARE, std::ptr::null_mut());
        let frames = (samples.len() / CHANNELS as usize) as libc::c_ulong;
        let mut x = Xferi {
            result: 0,
            buf: samples.as_ptr() as *const libc::c_void,
            frames,
        };
        let w = ioctl_ptr(
            self.fd.0,
            SNDRV_PCM_IOCTL_WRITEI_FRAMES,
            &mut x as *mut _ as *mut _,
        );
        if let Err(e) = w {
            if e.raw_os_error() == Some(libc::EPIPE) {
                // underrun: prepare and retry once
                ioctl_ptr(self.fd.0, SNDRV_PCM_IOCTL_PREPARE, std::ptr::null_mut())?;
                x.result = 0;
                ioctl_ptr(
                    self.fd.0,
                    SNDRV_PCM_IOCTL_WRITEI_FRAMES,
                    &mut x as *mut _ as *mut _,
                )
                .map_err(|e| Error::new(e.kind(), format!("WRITEI: {e}")))?;
            } else {
                return Err(Error::new(e.kind(), format!("WRITEI: {e}")));
            }
        }
        ioctl_ptr(self.fd.0, SNDRV_PCM_IOCTL_DRAIN, std::ptr::null_mut())
            .map_err(|e| Error::new(e.kind(), format!("DRAIN: {e}")))?;
        Ok(())
    }

    /// As [`Pcm::open_params`] but for an arbitrary playback device number (card 0). Used by
    /// the device sweep to compare the three earphone-capable front-ends: D0 (ABE
    /// MultiMedia1), D6 (ABE MultiMedia1 LP) and D8 (twl6040-legacy, McPDM direct, no ABE).
    pub fn open_params_dev(
        device: u32,
        period: u32,
        periods: u32,
        start_threshold: u32,
    ) -> io::Result<Pcm> {
        let fd = open_snd(&format!("pcmC{CARD}D{device}p"))?;
        let (period_size, buffer_size) = set_hw_params(&fd, period, periods)?;
        let st = if start_threshold == 0 {
            buffer_size
        } else {
            start_threshold.min(buffer_size)
        };
        set_sw_params_st(&fd, period_size, buffer_size, st)?;
        ioctl_ptr(fd.0, SNDRV_PCM_IOCTL_PREPARE, std::ptr::null_mut())
            .map_err(|e| Error::new(e.kind(), format!("PREPARE: {e}")))?;
        Ok(Pcm {
            fd,
            period_size,
            buffer_size,
        })
    }

    /// Opens device 0 with an explicit period/count and start threshold (frames). Used by
    /// the diagnostic to compare parameter sets. Leaves the stream PREPARED.
    pub fn open_params(period: u32, periods: u32, start_threshold: u32) -> io::Result<Pcm> {
        let fd = open_snd(&format!("pcmC{CARD}D{DEVICE}p"))?;
        let (period_size, buffer_size) = set_hw_params(&fd, period, periods)?;
        let st = if start_threshold == 0 {
            buffer_size
        } else {
            start_threshold.min(buffer_size)
        };
        set_sw_params_st(&fd, period_size, buffer_size, st)?;
        ioctl_ptr(fd.0, SNDRV_PCM_IOCTL_PREPARE, std::ptr::null_mut())
            .map_err(|e| Error::new(e.kind(), format!("PREPARE: {e}")))?;
        Ok(Pcm {
            fd,
            period_size,
            buffer_size,
        })
    }

    pub fn prepare(&self) -> io::Result<()> {
        ioctl_ptr(self.fd.0, SNDRV_PCM_IOCTL_PREPARE, std::ptr::null_mut())
            .map_err(|e| Error::new(e.kind(), format!("PREPARE: {e}")))
    }

    pub fn start(&self) -> io::Result<()> {
        ioctl_ptr(self.fd.0, SNDRV_PCM_IOCTL_START, std::ptr::null_mut())
            .map_err(|e| Error::new(e.kind(), format!("START: {e}")))
    }

    pub fn drain(&self) -> io::Result<()> {
        ioctl_ptr(self.fd.0, SNDRV_PCM_IOCTL_DRAIN, std::ptr::null_mut())
            .map_err(|e| Error::new(e.kind(), format!("DRAIN: {e}")))
    }

    /// One blocking interleaved write of `samples` (S16 stereo). Returns frames accepted.
    pub fn writei(&self, samples: &[i16]) -> io::Result<usize> {
        let frames = (samples.len() / CHANNELS as usize) as libc::c_ulong;
        let mut x = Xferi {
            result: 0,
            buf: samples.as_ptr() as *const libc::c_void,
            frames,
        };
        ioctl_ptr(
            self.fd.0,
            SNDRV_PCM_IOCTL_WRITEI_FRAMES,
            &mut x as *mut _ as *mut _,
        )
        .map_err(|e| Error::new(e.kind(), format!("WRITEI: {e}")))?;
        Ok(x.result as usize)
    }

    /// Plays `samples` with an **explicit START** after one period is queued: PREPARE,
    /// write one period, START, write the rest, DRAIN. This is the sequence that makes the
    /// OMAP4 ABE → McPDM → TWL6040 back-end actually clock from cold (confirmed on
    /// hardware: variant V5). Writing a full period before START (rather than relying on
    /// the start threshold) is what gets the DMA running on a cold codec.
    pub fn play_explicit(&self, samples: &[i16]) -> io::Result<()> {
        let ch = CHANNELS as usize;
        let total = samples.len() / ch;
        if total == 0 {
            return Ok(());
        }
        let one = (self.period_size as usize).min(total);
        self.prepare()?;
        self.writei(&samples[..one * ch])?;
        self.start()?;
        if one < total {
            self.writei(&samples[one * ch..])?;
        }
        self.drain()
    }
}

impl Drop for Pcm {
    fn drop(&mut self) {
        let _ = ioctl_ptr(self.fd.0, SNDRV_PCM_IOCTL_DROP, std::ptr::null_mut());
    }
}

fn set_hw_params(fd: &Fd, period_size: u32, periods: u32) -> io::Result<(u32, u32)> {
    set_hw_params_ex(
        fd,
        SNDRV_PCM_FORMAT_S16_LE,
        RATE,
        CHANNELS,
        period_size,
        periods,
    )
}

/// As [`set_hw_params`] but with an explicit format / rate / channel count — for the legacy
/// McPDM-direct path (D8), which is S32_LE at 88.2/96 kHz (OMAP_MCPDM_RATES, TWL6040_FORMATS).
fn set_hw_params_ex(
    fd: &Fd,
    format: u32,
    rate: u32,
    channels: u32,
    period_size: u32,
    periods: u32,
) -> io::Result<(u32, u32)> {
    let mut p = [0u8; SIZEOF_HW_PARAMS];
    // param_init: all masks full, all intervals [0, ~0], rmask/info all set.
    for k in 0..3 {
        put_u32(&mut p, HWP_MASK0 + k * 32, 0xffff_ffff);
        put_u32(&mut p, HWP_MASK0 + k * 32 + 4, 0xffff_ffff);
    }
    for i in 0..12 {
        put_u32(&mut p, HWP_INTERVAL0 + i * 12, 0); // min
        put_u32(&mut p, HWP_INTERVAL0 + i * 12 + 4, 0xffff_ffff); // max
    }
    put_u32(&mut p, HWP_RMASK, 0xffff_ffff);
    put_u32(&mut p, HWP_INFO, 0xffff_ffff);

    set_mask(&mut p, P_ACCESS, SNDRV_PCM_ACCESS_RW_INTERLEAVED);
    set_mask(&mut p, P_FORMAT, format);
    set_interval_exact(&mut p, P_CHANNELS, channels);
    set_interval_exact(&mut p, P_RATE, rate);
    if period_size > 0 {
        set_interval_exact(&mut p, P_PERIOD_SIZE, period_size);
    }
    if periods > 0 {
        set_interval_exact(&mut p, P_PERIODS, periods);
    }
    ioctl_ptr(fd.0, SNDRV_PCM_IOCTL_HW_PARAMS, p.as_mut_ptr() as *mut _)?;

    // Read back the kernel's chosen period and buffer (interval min field).
    let got_period = get_u32(&p, HWP_INTERVAL0 + (P_PERIOD_SIZE - FIRST_INTERVAL) * 12);
    let got_buffer = get_u32(&p, HWP_INTERVAL0 + (P_BUFFER_SIZE - FIRST_INTERVAL) * 12);
    let period = if got_period == 0 || got_period == 0xffff_ffff {
        1024
    } else {
        got_period
    };
    let buffer = if got_buffer == 0 || got_buffer == 0xffff_ffff {
        period * periods.max(2)
    } else {
        got_buffer
    };
    Ok((period, buffer))
}

fn set_sw_params(fd: &Fd, period_size: u32, buffer_size: u32) -> io::Result<()> {
    set_sw_params_st(fd, period_size, buffer_size, 1)
}

fn set_sw_params_st(
    fd: &Fd,
    period_size: u32,
    buffer_size: u32,
    start_threshold: u32,
) -> io::Result<()> {
    // 26 u32 words = 104 bytes.
    let mut w = [0u32; SIZEOF_SW_PARAMS / 4];
    // boundary: largest multiple of buffer_size below 2^31.
    let mut boundary = buffer_size.max(1);
    while boundary <= 0x4000_0000 {
        boundary = boundary.wrapping_mul(2);
    }
    boundary /= 2;
    w[0] = 0; // tstamp_mode = NONE
    w[1] = 1; // period_step
    w[2] = 0; // sleep_min
    w[3] = period_size; // avail_min
    w[4] = 1; // xfer_align (obsolete)
    w[5] = start_threshold.max(1); // start_threshold
    w[6] = buffer_size; // stop_threshold
    w[7] = 0; // silence_threshold
    w[8] = 0; // silence_size
    w[9] = boundary; // boundary
                     // w[10] proto, w[11] tstamp_type, rest reserved: 0
    ioctl_ptr(fd.0, SNDRV_PCM_IOCTL_SW_PARAMS, w.as_mut_ptr() as *mut _)
        .map_err(|e| Error::new(e.kind(), format!("SW_PARAMS: {e}")))
}

fn set_mask(p: &mut [u8], param: usize, value: u32) {
    let off = HWP_MASK0 + param * 32;
    for b in p[off..off + 32].iter_mut() {
        *b = 0;
    }
    let word = (value / 32) as usize;
    let bit = value % 32;
    put_u32(p, off + word * 4, 1u32 << bit);
}

fn set_interval_exact(p: &mut [u8], param: usize, value: u32) {
    let off = HWP_INTERVAL0 + (param - FIRST_INTERVAL) * 12;
    put_u32(p, off, value); // min
    put_u32(p, off + 4, value); // max
    put_u32(p, off + 8, 0x4); // flags: integer = 1 (bit 2)
}

fn put_u32(buf: &mut [u8], off: usize, v: u32) {
    buf[off..off + 4].copy_from_slice(&v.to_le_bytes());
}
fn put_i32(buf: &mut [u8], off: usize, v: i32) {
    buf[off..off + 4].copy_from_slice(&v.to_le_bytes());
}
fn get_u32(buf: &[u8], off: usize) -> u32 {
    u32::from_le_bytes(buf[off..off + 4].try_into().unwrap())
}
fn get_i32(buf: &[u8], off: usize) -> i32 {
    i32::from_le_bytes(buf[off..off + 4].try_into().unwrap())
}

// ---- sound synthesis ---------------------------------------------------------------

/// A short tone segment: a frequency held for `ms` milliseconds.
struct Seg {
    freq: f32,
    ms: f32,
}

/// Renders a sequence of tone segments to interleaved S16 stereo (same signal on both
/// channels: the EP output is mono, and a mono earbud gets a correct down-mix). A short
/// attack and an exponential decay on the whole sound avoid onset/offset clicks.
fn render(segs: &[Seg], peak: f32) -> Vec<i16> {
    let total_ms: f32 = segs.iter().map(|s| s.ms).sum();
    let total = (total_ms * RATE as f32 / 1000.0) as usize;
    let attack = (RATE as f32 * 0.0015) as usize; // 1.5 ms
    let mut out = Vec::with_capacity(total * 2);
    let mut phase = 0.0f32;
    let mut seg_i = 0usize;
    let mut seg_end = (segs[0].ms * RATE as f32 / 1000.0) as usize;
    let mut freq = segs[0].freq;
    for n in 0..total {
        while n >= seg_end && seg_i + 1 < segs.len() {
            seg_i += 1;
            seg_end += (segs[seg_i].ms * RATE as f32 / 1000.0) as usize;
            freq = segs[seg_i].freq;
        }
        phase += 2.0 * std::f32::consts::PI * freq / RATE as f32;
        // Envelope: linear attack, then exponential decay over the whole sound.
        let a = if n < attack {
            n as f32 / attack as f32
        } else {
            1.0
        };
        let decay = (-3.0 * n as f32 / total as f32).exp();
        let s = (phase.sin() * a * decay * peak * 32767.0) as i16;
        out.push(s);
        out.push(s);
    }
    out
}

/// The pre-rendered buffer for each UI cue.
pub struct Clips {
    card_change: Vec<i16>,
    select: Vec<i16>,
    menu_move: Vec<i16>,
    toggle_on: Vec<i16>,
    toggle_off: Vec<i16>,
}

impl Clips {
    pub fn new() -> Clips {
        Clips {
            card_change: render(
                &[Seg {
                    freq: 1800.0,
                    ms: 18.0,
                }],
                0.22,
            ),
            select: render(
                &[
                    Seg {
                        freq: 1200.0,
                        ms: 18.0,
                    },
                    Seg {
                        freq: 1600.0,
                        ms: 30.0,
                    },
                ],
                0.28,
            ),
            menu_move: render(
                &[Seg {
                    freq: 2300.0,
                    ms: 14.0,
                }],
                0.20,
            ),
            toggle_on: render(
                &[
                    Seg {
                        freq: 900.0,
                        ms: 20.0,
                    },
                    Seg {
                        freq: 1400.0,
                        ms: 28.0,
                    },
                ],
                0.26,
            ),
            toggle_off: render(
                &[
                    Seg {
                        freq: 1400.0,
                        ms: 20.0,
                    },
                    Seg {
                        freq: 900.0,
                        ms: 28.0,
                    },
                ],
                0.26,
            ),
        }
    }

    fn for_cue(&self, cue: Cue) -> &[i16] {
        match cue {
            Cue::CardChange => &self.card_change,
            Cue::Select | Cue::VolumeStep => &self.select,
            Cue::MenuMove => &self.menu_move,
            Cue::ToggleOn => &self.toggle_on,
            Cue::ToggleOff => &self.toggle_off,
        }
    }
}

impl Default for Clips {
    fn default() -> Self {
        Self::new()
    }
}

/// A clear, sustained single-frequency tone: `freq` Hz for `ms` milliseconds at `peak`
/// (0..1 of full scale), with short linear attack/release so it is unmistakable but does
/// not click. Interleaved S16 stereo (same on both channels). For the diagnostic sweep.
/// `n` short tones of `ms` each separated by `gap_ms` of silence (interleaved stereo).
pub fn beeps(n: usize, freq: f32, ms: f32, gap_ms: f32, peak: f32) -> Vec<i16> {
    let gap = (gap_ms * RATE as f32 / 1000.0) as usize * 2;
    let one = tone(freq, ms, peak);
    let mut out = Vec::with_capacity((one.len() + gap) * n);
    for i in 0..n {
        if i > 0 {
            out.extend(std::iter::repeat(0i16).take(gap));
        }
        out.extend_from_slice(&one);
    }
    out
}

pub fn tone(freq: f32, ms: f32, peak: f32) -> Vec<i16> {
    let total = (ms * RATE as f32 / 1000.0) as usize;
    let edge = (RATE as f32 * 0.008) as usize; // 8 ms attack / release
    let mut out = Vec::with_capacity(total * 2);
    let mut phase = 0.0f32;
    for n in 0..total {
        phase += 2.0 * std::f32::consts::PI * freq / RATE as f32;
        let env = if n < edge {
            n as f32 / edge as f32
        } else if n >= total - edge {
            (total - n) as f32 / edge as f32
        } else {
            1.0
        };
        let s = (phase.sin() * env * peak * 32767.0) as i16;
        out.push(s);
        out.push(s);
    }
    out
}

/// The startup chime: an ascending major arpeggio, Do-Mi-Sol-Do (C5 E5 G5 C6), the
/// owner's choice. Each note is a clean sine with a short fade so nothing clicks; the
/// last note is held longer. Moderate level: clean on the earpiece at 0 dB without
/// clipping the bone-conduction amp.
pub fn startup_tone() -> Vec<i16> {
    const PEAK: f32 = 0.45;
    const GAP_MS: f32 = 35.0;
    let notes: [(f32, f32); 4] = [
        (523.25, 230.0),  // C5  do
        (659.25, 230.0),  // E5  mi
        (783.99, 230.0),  // G5  sol
        (1046.50, 520.0), // C6  do (alto)
    ];
    let gap = (GAP_MS * RATE as f32 / 1000.0) as usize * 2;
    let mut out = Vec::new();
    for (i, (freq, ms)) in notes.iter().enumerate() {
        if i > 0 {
            out.extend(std::iter::repeat(0i16).take(gap));
        }
        out.extend(tone(*freq, *ms, PEAK));
    }
    out
}

// ---- the installed Feedback player -------------------------------------------------

/// The on-device feedback player: a background audio thread owns the PCM and mixer, so
/// [`Feedback::play`] only sends a cue and returns at once (never blocks the input loop).
pub struct AlsaFeedback {
    tx: mpsc::Sender<Snd>,
    /// The wanted earpiece volume (`u32::MAX`: not set yet); the audio thread applies it.
    volume: std::sync::Arc<std::sync::atomic::AtomicU32>,
}

/// Playback period/count for the UI player: 1056 frames × 4 = low latency (~22 ms period,
/// ~88 ms buffer) for snappy cues. Confirmed working on hardware once primed.
const UI_PERIOD: u32 = 1056;
const UI_PERIODS: u32 = 4;

impl AlsaFeedback {
    /// Opens the mixer, applies the cold-start priming set ([`prime_controls`], the "V5"
    /// recipe), opens the PCM (1056×4), and **primes the codec once** by playing a short
    /// startup tone with an explicit START (the step that makes the bone-conduction path
    /// clock from cold). Then starts the audio thread, which plays each cue on the primed
    /// path. Returns the player and a human-readable report (for the log).
    pub fn start() -> (Option<AlsaFeedback>, Vec<String>) {
        let mut report = Vec::new();
        let mixer = match Mixer::open() {
            Ok(m) => m,
            Err(e) => {
                report.push(format!("mixer open failed: {e}"));
                return (None, report);
            }
        };
        let headset = headset_present();
        report.push(format!(
            "route: {}",
            if headset {
                "USB earbud (Headset L/R)"
            } else {
                "bone conduction (Earphone/EP)"
            }
        ));
        // Apply the full priming controls BEFORE opening the front-end (route-first), so
        // DPCM links the McPDM back-end at open.
        for (name, val, r) in set_prime(&mixer, headset) {
            match r {
                Ok(()) => report.push(format!("mixer {name} = {val}: ok")),
                Err(e) => report.push(format!("mixer {name} = {val}: {e}")),
            }
        }
        // Full EP/codec mixer read-back, so if the tone is still inaudible we can see the
        // actual analog path state (what to check next: HP mode, gains, PDM routing).
        for line in ep_state_report(&mixer) {
            report.push(line);
        }
        let pcm = match Pcm::open_params(UI_PERIOD, UI_PERIODS, 0) {
            Ok(p) => {
                report.push(format!(
                    "pcm card{CARD} dev{DEVICE}: open ok (period {} buffer {} frames @ {RATE} Hz)",
                    p.period_size, p.buffer_size
                ));
                p
            }
            Err(e) => {
                report.push(format!("pcm open failed: {e}"));
                return (None, report);
            }
        };
        // Cold-prime + startup tone: one explicit-START play clocks the codec. After this,
        // every per-cue play drains normally.
        match pcm.play_explicit(&startup_tone()) {
            Ok(()) => report.push("codec primed (startup tone played, explicit START)".into()),
            Err(e) => report.push(format!("prime/startup tone FAILED: {e}")),
        }

        let (tx, rx) = mpsc::channel::<Snd>();
        let volume = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(u32::MAX));
        let thread_volume = volume.clone();
        thread::Builder::new()
            .name("ui-audio".into())
            .spawn(move || {
                use std::time::{Duration, Instant};
                // Minimum gap between repeatable ticks (CardChange/MenuMove), so a fast
                // scroll or any rapid repeat collapses to at most one tick per interval
                // instead of stacking into a buzz. Discrete cues (Select/Toggle) and the
                // chime always play. Each play is prepare->write->START->drain, so the PCM
                // is drained (silent) between cues; nothing is emitted while idle.
                const MIN_GAP: Duration = Duration::from_millis(70);
                let clips = Clips::new();
                let chime = startup_tone();
                // The mixer stays open for the session: the volume is set from here.
                let mut applied = u32::MAX;
                let mut last = Instant::now() - Duration::from_secs(1);
                while let Ok(mut snd) = rx.recv() {
                    // Coalesce a backlog (fast swipes) down to the latest sound.
                    while let Ok(newer) = rx.try_recv() {
                        snd = newer;
                    }
                    let repeatable = matches!(
                        snd,
                        Snd::Cue(Cue::CardChange | Cue::MenuMove | Cue::VolumeStep)
                    );
                    if repeatable && last.elapsed() < MIN_GAP {
                        continue;
                    }
                    last = Instant::now();
                    let wanted = thread_volume.load(std::sync::atomic::Ordering::Relaxed);
                    if wanted != applied && wanted != u32::MAX {
                        let (ep, media) = volume_gains(wanted);
                        let _ = mixer.set("Earphone Playback Volume", ep);
                        let _ = mixer.set("DL1 Media Playback Volume", media);
                        applied = wanted;
                    }
                    let buf: &[i16] = match snd {
                        Snd::Cue(cue) => clips.for_cue(cue),
                        Snd::Chime => &chime,
                    };
                    let _ = pcm.play_explicit(buf);
                }
            })
            .ok();
        report.push("audio thread started".into());
        (Some(AlsaFeedback { tx, volume }), report)
    }

    /// Plays the startup chime (for the self-test audibility check).
    pub fn play_chime(&mut self) {
        let _ = self.tx.send(Snd::Chime);
    }
}

/// The mixer values for a volume in percent: (`Earphone Playback Volume`, `DL1 Media Playback
/// Volume`). The earpiece stays at its +6 dB maximum, and the ABE gain (120 = 0 dB, taken as
/// 1 dB per step) does the fine control: 100 % is +6 dB overall, each percent 0.46 dB less, and
/// 0 is the bottom of the ABE range. The 1 dB step has not been measured on the device.
pub fn volume_gains(percent: u32) -> (i32, i32) {
    const EP_MAX: i32 = 15;
    const ABE_0_DB: f32 = 120.0;
    const DB_PER_PERCENT: f32 = 0.46;
    if percent == 0 {
        return (EP_MAX, 0);
    }
    let below_max = 100u32.saturating_sub(percent) as f32 * DB_PER_PERCENT;
    (EP_MAX, (ABE_0_DB - below_max).round() as i32)
}

#[cfg(test)]
mod volume_tests {
    use super::volume_gains;

    #[test]
    fn the_volume_falls_steadily_and_0_is_the_bottom() {
        assert_eq!(volume_gains(100), (15, 120));
        assert_eq!(volume_gains(87), (15, 114));
        assert_eq!(volume_gains(1), (15, 74));
        assert_eq!(volume_gains(0), (15, 0));
        for p in 1..=100 {
            assert!(volume_gains(p).1 >= volume_gains(p - 1).1);
        }
    }
}

/// A queued sound for the audio thread.
enum Snd {
    Cue(Cue),
    Chime,
}

impl Feedback for AlsaFeedback {
    fn play(&mut self, cue: Cue) {
        let _ = self.tx.send(Snd::Cue(cue));
    }

    fn set_volume(&mut self, percent: u32) {
        self.volume
            .store(percent, std::sync::atomic::Ordering::Relaxed);
    }
}

// ---- audio self-test (for the init's startup tone) ---------------------------------

/// Applies the cold-start priming set ([`prime_controls`]) and plays a startup tone with
/// an explicit START (the "V5" recipe), returning a log of each step. Used by
/// `glass-launcher --audio-selftest` from the init. `true` if a tone was played.
pub fn selftest() -> (bool, Vec<String>) {
    let mut log = Vec::new();
    let mixer = match Mixer::open() {
        Ok(m) => {
            log.push("controlC0 (mixer) open: ok".into());
            m
        }
        Err(e) => {
            log.push(format!("controlC0 open: FAIL: {e}"));
            return (false, log);
        }
    };
    let headset = headset_present();
    log.push(format!("headset detected: {headset}"));
    for (name, val, r) in set_prime(&mixer, headset) {
        match r {
            Ok(()) => log.push(format!("mixer {name} = {val}: ok")),
            Err(e) => log.push(format!("mixer {name} = {val}: FAIL: {e}")),
        }
    }
    let pcm = match Pcm::open_params(UI_PERIOD, UI_PERIODS, 0) {
        Ok(p) => {
            log.push(format!(
                "pcmC0D0p open: ok (period {} buffer {} frames @ {RATE} Hz S16_LE stereo)",
                p.period_size, p.buffer_size
            ));
            p
        }
        Err(e) => {
            log.push(format!("pcmC0D0p open: FAIL: {e}"));
            return (false, log);
        }
    };
    match pcm.play_explicit(&startup_tone()) {
        Ok(()) => {
            log.push("startup tone: played (prime + explicit START + drain ok)".into());
            (true, log)
        }
        Err(e) => {
            log.push(format!("startup tone: FAIL: {e}"));
            (false, log)
        }
    }
}

// ---- verbose on-screen diagnostic (for the fbcon diag image) -----------------------

fn diag_play(emit: &mut dyn FnMut(&str), pcm: &Pcm, label: &str, buf: &[i16]) {
    let frames = buf.len() / CHANNELS as usize;
    match pcm.play_explicit(buf) {
        Ok(()) => emit(&format!(
            "{label}: wrote {frames} frames ({} bytes), explicit-START+drain ok",
            frames * CHANNELS as usize * 2
        )),
        Err(e) => emit(&format!("{label}: PLAY FAIL: {e}{}", errno_suffix(&e))),
    }
}

/// Applies `controls` on the mixer and reads each back, reporting written-vs-stuck values.
fn diag_apply(emit: &mut dyn FnMut(&str), mixer: &Mixer, controls: &[(&'static str, i32)]) {
    for &(name, val) in controls {
        let w = mixer.set(name, val);
        let rb = mixer.get(name);
        match (w, rb) {
            (Ok(()), Ok(v)) => emit(&format!("  set {name} = {val}; readback = {v:?}")),
            (Ok(()), Err(e)) => emit(&format!("  set {name} = {val}; readback FAIL: {e}")),
            (Err(e), _) => emit(&format!(
                "  set {name} = {val}: FAIL: {e}{}",
                errno_suffix(&e)
            )),
        }
    }
}

/// One full verbose audio diagnostic pass, emitting each line through `emit` as it goes
/// (so the init can show it on the framebuffer console in real time) and sleeping between
/// tones so a human can read and listen. Steps a–g from the diagnosis request:
/// dump cards + /dev/snd + the mixer control list; set the bone-conduction (EP) route and
/// read it back; open the PCM (ABE period first, then fall-backs); play a clear 1 kHz
/// tone; sweep the Earphone volume; then repeat on the USB-earbud route; finally a summary.
pub fn diag(emit: &mut dyn FnMut(&str)) {
    use std::time::Duration;
    let sl = |ms: u64| std::thread::sleep(Duration::from_millis(ms));

    emit("================ OMGB AUDIO DIAGNOSTIC ================");

    // a. sound card + PCM node inventory
    match fs::read_to_string("/proc/asound/cards") {
        Ok(s) => {
            for line in s.lines() {
                emit(&format!("asound: {line}"));
            }
        }
        Err(e) => emit(&format!("/proc/asound/cards: {e}")),
    }
    match fs::read_dir("/dev/snd") {
        Ok(rd) => {
            let mut names: Vec<String> = rd
                .filter_map(|e| e.ok().map(|e| e.file_name().to_string_lossy().into_owned()))
                .collect();
            names.sort();
            emit(&format!("/dev/snd nodes: {}", names.join(" ")));
        }
        Err(e) => emit(&format!("/dev/snd: {e}")),
    }
    emit(&format!(
        "headset (usb_mux) detected: {}",
        headset_present()
    ));

    // open the mixer and dump every control (exact names, for diagnosis)
    let mixer = match Mixer::open() {
        Ok(m) => {
            emit("controlC0 (mixer) open: ok");
            m
        }
        Err(e) => {
            emit(&format!("controlC0 open: FAIL: {e} -- cannot continue"));
            return;
        }
    };
    match mixer.list() {
        Ok(ctls) => {
            emit(&format!("mixer exposes {} controls:", ctls.len()));
            for (numid, iface, name) in &ctls {
                emit(&format!("  ctl#{numid} iface{iface} {name:?}"));
            }
        }
        Err(e) => emit(&format!("mixer control list: FAIL: {e}")),
    }

    // --- ROUTE 1: bone conduction (EP) ---
    // Prime the route BEFORE opening the front-end (route-first) so the McPDM back-end is
    // linked at open and the codec clocks from cold (the hardware-confirmed order).
    emit("---- ROUTE 1: BONE CONDUCTION (TWL6040 Earphone / EP) ----");
    diag_apply(emit, &mixer, &prime_controls(false));
    // c. open the playback PCM once (reused for both routes)
    let pcm = match Pcm::open_diag(emit) {
        Ok(p) => p,
        Err(e) => {
            emit(&format!("PCM open FAILED: {e} -- no tones can play"));
            return;
        }
    };
    emit("playing a clear 1 kHz tone (1.5 s) on BONE CONDUCTION ...");
    diag_play(emit, &pcm, "EP tone", &tone(1000.0, 1500.0, 0.25));
    sl(500);
    emit("-- EP VOLUME SWEEP: Earphone Playback Volume stepping to max (15) --");
    for v in [6, 9, 12, 14, 15] {
        let w = mixer.set("Earphone Playback Volume", v);
        let rb = mixer.get("Earphone Playback Volume");
        emit(&format!(
            ">>> EP volume = {v} (set {w:?}, readback {rb:?}) -- ASCOLTA ORA (bone)"
        ));
        sl(400);
        diag_play(
            emit,
            &pcm,
            &format!("EP vol {v} tone"),
            &tone(1000.0, 1000.0, 0.3),
        );
        sl(600);
    }

    // --- ROUTE 2: USB earbud / headset ---
    emit("---- ROUTE 2: AURICOLARE/USB -- ATTACCA L'AURICOLARE SE VUOI ----");
    diag_apply(emit, &mixer, &prime_controls(true));
    emit("playing a clear 1 kHz tone (1.5 s) on HEADSET ...");
    diag_play(emit, &pcm, "HS tone", &tone(1000.0, 1500.0, 0.25));
    sl(500);
    emit("-- HEADSET VOLUME SWEEP: Headset Playback Volume stepping to max (15) --");
    for v in [8, 11, 13, 15] {
        let w = mixer.set("Headset Playback Volume", v);
        let rb = mixer.get("Headset Playback Volume");
        emit(&format!(
            ">>> HS volume = {v} (set {w:?}, readback {rb:?}) -- ASCOLTA ORA (auricolare)"
        ));
        sl(400);
        diag_play(
            emit,
            &pcm,
            &format!("HS vol {v} tone"),
            &tone(1000.0, 1000.0, 0.3),
        );
        sl(600);
    }

    // g. summary
    emit(&format!(
        "SUMMARY: PCM period {} buffer {} @ {RATE} Hz S16_LE stereo; played EP(sweep 6..15) then HEADSET(sweep 8..15). Any 'FAIL'/'errno' line above marks the stage that broke.",
        pcm.period_size, pcm.buffer_size
    ));
    emit("================ END OF DIAGNOSTIC PASS ================");
}

// ---- diag2: find the missing back-end (McPDM/TWL6040) bring-up ----------------------

/// How a variant orders mixer-route vs PCM-open, and how it starts the stream.
struct Variant {
    label: &'static str,
    /// Set the DAPM route/mixers BEFORE opening the PCM (true) or AFTER (false, the old
    /// broken order).
    route_first: bool,
    /// Extra controls to apply before opening (codec power / PLL), name→value.
    extra: &'static [(&'static str, i32)],
    period: u32,
    periods: u32,
    /// Fill one period, issue SNDRV_PCM_IOCTL_START, then write the rest (true); else rely
    /// on the start threshold to auto-start on the first write (false).
    explicit_start: bool,
}

/// Runs one playback variant on the bone-conduction (EP) route and reports the result +
/// elapsed time (a fast ~1 s = the DMA drained = SOUND; a ~19 s stall + EIO = back-end
/// never clocked). Each variant is fully self-contained (own mixer route + PCM open).
fn run_variant(emit: &mut dyn FnMut(&str), mixer: &Mixer, v: &Variant) {
    use std::time::Instant;
    emit(&format!("######## VARIANT {} ########", v.label));

    let apply_controls = |emit: &mut dyn FnMut(&str)| {
        for (name, val) in v.extra {
            match mixer.set(name, *val) {
                Ok(()) => {
                    let rb = mixer.get(name).ok();
                    emit(&format!("  pre: {name} = {val} (readback {rb:?})"));
                }
                Err(e) => emit(&format!("  pre: {name} = {val}: FAIL: {e}")),
            }
        }
        diag_apply(emit, mixer, &route_controls(false)); // EP / bone-conduction route
    };

    if v.route_first {
        apply_controls(emit);
    }
    let pcm = match Pcm::open_params(v.period, v.periods, if v.explicit_start { 0 } else { 1 }) {
        Ok(p) => {
            emit(&format!(
                "  open ok: period {} buffer {} (requested {}/{})",
                p.period_size, p.buffer_size, v.period, v.periods
            ));
            p
        }
        Err(e) => {
            emit(&format!("  open FAIL: {e}{}", errno_suffix(&e)));
            return;
        }
    };
    if !v.route_first {
        apply_controls(emit);
    }

    // ~0.8 s 1 kHz tone, interleaved stereo.
    let buf = tone(1000.0, 800.0, 0.3);
    let total = buf.len() / CHANNELS as usize;
    let t0 = Instant::now();
    let result: io::Result<()> = (|| {
        if v.explicit_start {
            // Fill one period, start, then write the remainder.
            let one = (pcm.period_size as usize).min(total);
            pcm.prepare()?;
            pcm.writei(&buf[..one * CHANNELS as usize])?;
            pcm.start()?;
            if one < total {
                pcm.writei(&buf[one * CHANNELS as usize..])?;
            }
        } else {
            pcm.prepare()?;
            pcm.writei(&buf)?; // auto-starts at the threshold
        }
        pcm.drain()
    })();
    let ms = t0.elapsed().as_millis();
    match result {
        Ok(()) => emit(&format!(
            "  >>> PLAY OK in {ms} ms ({total} frames) -- SE SENTI UN TONO, QUESTA VARIANTE FUNZIONA ({})",
            v.label
        )),
        Err(e) => emit(&format!(
            "  >>> PLAY FAIL in {ms} ms: {e}{}",
            errno_suffix(&e)
        )),
    }
    // pcm dropped here (DROP ioctl) before the next variant.
}

/// Back-end bring-up experiment: try several orderings / periods / start methods / codec
/// power-mode presets on the bone-conduction (EP) path, one per boot pass, so the log (and
/// the ear) tell us which makes the DMA actually drain. Steps between variants with a gap.
pub fn diag2(emit: &mut dyn FnMut(&str)) {
    use std::time::Duration;
    let sl = |ms: u64| std::thread::sleep(Duration::from_millis(ms));

    emit("============ OMGB AUDIO DIAG2 (back-end bring-up) ============");
    match fs::read_to_string("/proc/asound/cards") {
        Ok(s) => {
            for line in s.lines() {
                emit(&format!("asound: {line}"));
            }
        }
        Err(e) => emit(&format!("/proc/asound/cards: {e}")),
    }
    let mixer = match Mixer::open() {
        Ok(m) => {
            emit("controlC0 open: ok");
            m
        }
        Err(e) => {
            emit(&format!("controlC0 open FAIL: {e} -- abort"));
            return;
        }
    };

    // Codec power/PLL presets to try (enum item indices: 0=Low-Power, 1=High-Performance).
    const HP: &[(&str, i32)] = &[
        ("PLL Selection", 1),
        ("TWL6040 Power Mode", 1),
        ("Headset Power Mode", 1),
    ];
    // Full stock mixer table extras (spec 4.2), in case a widget is missing from the path.
    const STOCK: &[(&str, i32)] = &[
        ("PLL Selection", 1),
        ("TWL6040 Power Mode", 1),
        ("Headset Power Mode", 1),
        ("DL1 Mono Mixer", 0),
        ("Sidetone Mixer Playback", 1),
        ("DL1 Mixer Voice", 0),
    ];

    let variants = [
        // V0: the OLD (broken) order — route set AFTER opening the FE. Control case.
        Variant {
            label: "V0 old-order (route AFTER open), 14784/2, autostart",
            route_first: false,
            extra: &[],
            period: 14784,
            periods: 2,
            explicit_start: false,
        },
        // V1: route FIRST, small non-mmap period (spec 7 recommendation), autostart.
        Variant {
            label: "V1 route-FIRST, 1056/4, autostart",
            route_first: true,
            extra: &[],
            period: 1056,
            periods: 4,
            explicit_start: false,
        },
        // V2: route FIRST, small period, explicit PREPARE+START after one period.
        Variant {
            label: "V2 route-FIRST, 1056/4, explicit START",
            route_first: true,
            extra: &[],
            period: 1056,
            periods: 4,
            explicit_start: true,
        },
        // V3: route FIRST, ABE period, explicit START.
        Variant {
            label: "V3 route-FIRST, 14784/2, explicit START",
            route_first: true,
            extra: &[],
            period: 14784,
            periods: 2,
            explicit_start: true,
        },
        // V4: route FIRST + codec HP power/PLL preset, small period, explicit START.
        Variant {
            label: "V4 route-FIRST + PLL/PowerMode HP, 1056/4, explicit START",
            route_first: true,
            extra: HP,
            period: 1056,
            periods: 4,
            explicit_start: true,
        },
        // V5: route FIRST + full stock mixer table + HP, small period, explicit START.
        Variant {
            label: "V5 route-FIRST + stock table + HP, 1056/4, explicit START",
            route_first: true,
            extra: STOCK,
            period: 1056,
            periods: 4,
            explicit_start: true,
        },
    ];

    for v in &variants {
        run_variant(emit, &mixer, v);
        sl(1200); // gap so a working tone is clearly separated from the next
    }

    emit("DIAG2 SUMMARY: read which VARIANT shows 'PLAY OK' in ~1 s (and you heard a tone).");
    emit("A ~19 s stall + 'PLAY FAIL ... EIO' = that variant's back-end still did not clock.");
    emit("================ END OF DIAG2 PASS ================");
}

// ---- audibility sweep (amplitude × Earphone volume) --------------------------------

/// A primed EP path for the audibility sweep: the mixer (to set `Earphone Playback
/// Volume` per step) and the open PCM (to play a tone at a chosen synth amplitude). The
/// digital path is known-good; this finds a CLEAN, audible analog level — a loud synth at
/// the +6 dB cap clips ("pessimo"), so we sweep amplitude × EP volume.
pub struct AudioSweep {
    mixer: Mixer,
    pcm: Pcm,
}

impl AudioSweep {
    /// The amplitudes and Earphone volumes the sweep walks (outer × inner).
    pub const AMPS: [f32; 4] = [0.2, 0.4, 0.6, 0.9];
    pub const VOLUMES: [i32; 3] = [10, 13, 15];

    /// Opens the mixer, applies the bone-conduction priming route, and opens + cold-primes
    /// the PCM. Returns the sweep handle and a report (mixer read-back incl. EP state).
    pub fn open() -> (Option<AudioSweep>, Vec<String>) {
        let mut report = Vec::new();
        let mixer = match Mixer::open() {
            Ok(m) => m,
            Err(e) => {
                report.push(format!("mixer open failed: {e}"));
                return (None, report);
            }
        };
        for (name, val, r) in set_prime(&mixer, false) {
            if let Err(e) = r {
                report.push(format!("mixer {name} = {val}: {e}"));
            }
        }
        for line in ep_state_report(&mixer) {
            report.push(line);
        }
        let pcm = match Pcm::open_params(UI_PERIOD, UI_PERIODS, 0) {
            Ok(p) => p,
            Err(e) => {
                report.push(format!("pcm open failed: {e}"));
                return (None, report);
            }
        };
        if let Err(e) = pcm.play_explicit(&tone(1000.0, 200.0, 0.2)) {
            report.push(format!("prime play failed: {e}"));
        } else {
            report.push("codec primed for sweep".into());
        }
        (Some(AudioSweep { mixer, pcm }), report)
    }

    /// Sets the Earphone Playback Volume (0..15).
    pub fn set_ep_volume(&self, v: i32) -> io::Result<()> {
        self.mixer.set("Earphone Playback Volume", v)
    }

    /// Plays a steady tone at `amp` (0..1) for `ms` on the current route.
    pub fn play(&self, freq: f32, ms: f32, amp: f32) -> io::Result<()> {
        self.pcm.play_explicit(&tone(freq, ms, amp))
    }
}

// ---- TWL6040 analog-path diagnostic (earphone output) ------------------------------

/// Reads the REAL TWL6040 hardware registers over I2C (the regmap debugfs does not exist
/// on this build), decodes the earphone/HSDAC/charge-pump bits, and dumps the EP-path ASoC
/// DAPM widget power — so a device readback shows exactly what the analog stage is doing
/// during playback (and whether the DAPM "On" cache matches the real register).
fn dump_codec_state(emit: &mut dyn FnMut(&str)) {
    dump_twl6040_i2c(emit);
    dump_codec_reg_debugfs(emit);
    dump_dapm(emit);
}

const I2C_SLAVE: libc::c_ulong = 0x0703;
const I2C_SMBUS: libc::c_ulong = 0x0720;
const I2C_SMBUS_READ: u8 = 1;
const I2C_SMBUS_BYTE_DATA: u32 = 2;

#[repr(C)]
union I2cSmbusData {
    byte: u8,
    word: u16,
    block: [u8; 34],
}

#[repr(C)]
struct I2cSmbusIoctlData {
    read_write: u8,
    command: u8,
    size: u32,
    data: *mut I2cSmbusData,
}

/// Finds the TWL6040's I2C bus and address from sysfs (`/sys/bus/i2c/devices/<bus>-<addr>`
/// whose `name` is `twl6040`).
fn find_twl6040_i2c() -> Option<(u32, u16)> {
    for e in fs::read_dir("/sys/bus/i2c/devices").ok()?.flatten() {
        let name = fs::read_to_string(e.path().join("name")).unwrap_or_default();
        if name.trim() == "twl6040" {
            let base = e.file_name().to_string_lossy().into_owned();
            let (bus, addr) = base.split_once('-')?;
            return Some((bus.parse().ok()?, u16::from_str_radix(addr, 16).ok()?));
        }
    }
    None
}

fn i2c_read_reg(fd: RawFd, reg: u8) -> io::Result<u8> {
    let mut data = I2cSmbusData { block: [0; 34] };
    let mut args = I2cSmbusIoctlData {
        read_write: I2C_SMBUS_READ,
        command: reg,
        size: I2C_SMBUS_BYTE_DATA,
        data: &mut data,
    };
    let rc = unsafe { libc::ioctl(fd, I2C_SMBUS as _, &mut args as *mut _) };
    if rc < 0 {
        Err(Error::last_os_error())
    } else {
        Ok(unsafe { data.byte })
    }
}

/// Dumps the key TWL6040 registers read directly over I2C, with per-bit decode.
fn dump_twl6040_i2c(emit: &mut dyn FnMut(&str)) {
    let Some((bus, addr)) = find_twl6040_i2c() else {
        emit("twl6040: not found under /sys/bus/i2c/devices (cannot read registers over I2C)");
        return;
    };
    emit(&format!(
        "twl6040 on i2c-{bus} addr 0x{addr:02x}: reading registers directly"
    ));
    let path = format!("/dev/i2c-{bus}");
    let fd = match open_snd_like(&path, bus) {
        Ok(fd) => fd,
        Err(e) => {
            emit(&format!("  open {path}: {e}"));
            return;
        }
    };
    if unsafe { libc::ioctl(fd.0, I2C_SLAVE as _, addr as libc::c_ulong) } < 0 {
        emit(&format!(
            "  I2C_SLAVE 0x{addr:02x}: {}",
            Error::last_os_error()
        ));
        return;
    }
    // (addr, name) of the registers that matter for the earphone analog path.
    let regs: &[(u8, &str)] = &[
        (0x02, "ASICREV"),
        (0x05, "NCPCTL"),
        (0x06, "LDOCTL"),
        (0x07, "HPPLLCTL"),
        (0x08, "LPPLLCTL"),
        (0x09, "LPPLLDIV"),
        (0x0F, "LINEGAIN"),
        (0x10, "HSLCTL"),
        (0x11, "HSRCTL"),
        (0x12, "HSGAIN"),
        (0x13, "EARCTL"),
        (0x2E, "STATUS"),
    ];
    for &(reg, name) in regs {
        match i2c_read_reg(fd.0, reg) {
            Ok(v) => emit(&format!(
                "  R{reg:#04x} {name:<9} = 0x{v:02x} {}",
                decode_reg(reg, v)
            )),
            Err(e) => emit(&format!("  R{reg:#04x} {name:<9} = read FAIL: {e}")),
        }
    }
}

/// Human decode of the earphone-relevant registers (per twl6040.h).
fn decode_reg(reg: u8, v: u8) -> String {
    match reg {
        // NCPCTL: bit0 NCPENA (negative charge pump), bit1 NCPOPEN.
        0x05 => format!("[NCPENA={} NCPOPEN={}]", v & 1, (v >> 1) & 1),
        // LDOCTL: bit0 LSLDOENA, bit1 HSLDOENA, bit2 REFENA, bit4 LSLDO...
        0x06 => format!(
            "[LSLDOENA={} HSLDOENA={} REFENA={}]",
            v & 1,
            (v >> 1) & 1,
            (v >> 2) & 1
        ),
        // HSLCTL/HSRCTL: bit0 HSDACENA, bit1 HSDACMODE, bit2 HSDRVENA, bit3 HSDRVMODE.
        0x10 | 0x11 => format!(
            "[HSDACENA={} HSDACMODE={} HSDRVENA={} HSDRVMODE={}]",
            v & 1,
            (v >> 1) & 1,
            (v >> 2) & 1,
            (v >> 3) & 1
        ),
        // EARCTL: bit0 EARENA, bits1-4 EARGAIN.
        0x13 => format!("[EARENA={} EARGAIN={}]", v & 1, (v >> 1) & 0x0f),
        // HPPLL: bit0 HPLLENA, bit2 HPLLSQRBP, bit4 HPLLSQRENA.
        0x07 => format!("[HPLLENA={}]", v & 1),
        // LPPLL: bit0 LPLLENA, bit4 LPLLFIN.
        0x08 => format!("[LPLLENA={}]", v & 1),
        // STATUS: bit0 PLUGCOMP, bit3 VIBLOCDET, bit6 TSHUTDET... bit? ready.
        0x2e => format!("[raw bits set: {v:08b}]"),
        _ => String::new(),
    }
}

/// Reads the ASoC codec_reg debugfs file (driver's own register view), if present.
fn dump_codec_reg_debugfs(emit: &mut dyn FnMut(&str)) {
    fn find(dir: &std::path::Path, depth: u32, out: &mut Vec<std::path::PathBuf>) {
        if depth > 4 {
            return;
        }
        let Ok(rd) = fs::read_dir(dir) else { return };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                find(&p, depth + 1, out);
            } else if e.file_name() == "codec_reg" {
                out.push(p);
            }
        }
    }
    let mut files = Vec::new();
    find(
        std::path::Path::new("/sys/kernel/debug/asoc"),
        0,
        &mut files,
    );
    for f in files {
        if let Ok(s) = fs::read_to_string(&f) {
            emit(&format!(
                "codec_reg ({}): {}",
                f.display(),
                s.replace('\n', " ")
            ));
        }
    }
}

/// Opens a char device, creating the node from `/sys/class/i2c-dev/i2c-<bus>/dev` if the
/// device tmpfs has none (mdev may not have created it).
fn open_snd_like(path: &str, bus: u32) -> io::Result<Fd> {
    if let Some(fd) = try_open(path) {
        return Ok(fd);
    }
    let dev = fs::read_to_string(format!("/sys/class/i2c-dev/i2c-{bus}/dev"))?;
    let (maj, min) = dev
        .trim()
        .split_once(':')
        .and_then(|(a, b)| Some((a.parse::<u32>().ok()?, b.parse::<u32>().ok()?)))
        .ok_or_else(|| Error::other("bad i2c dev"))?;
    let c = CString::new(path).unwrap();
    unsafe { libc::mknod(c.as_ptr(), libc::S_IFCHR | 0o600, libc::makedev(maj, min)) };
    try_open(path).ok_or_else(|| Error::other(format!("open {path}: {}", Error::last_os_error())))
}

/// Emits the power state of the EP-path ASoC DAPM widgets (handles widget names with
/// spaces correctly, unlike a shell loop).
fn dump_dapm(emit: &mut dyn FnMut(&str)) {
    const WIDGETS: &[&str] = &[
        "HSDAC Left",
        "HSDAC Right",
        "HSDAC Power",
        "Earphone Driver",
        "Earphone Playback",
        "EP",
        "HS Left Driver",
        "Headset Left Playback",
        "Headset Right Playback",
        "PDM_DL1",
        "DL1",
    ];
    fn walk(dir: &std::path::Path, depth: u32, emit: &mut dyn FnMut(&str), widgets: &[&str]) {
        if depth > 4 {
            return;
        }
        let Ok(rd) = fs::read_dir(dir) else { return };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, depth + 1, emit, widgets);
            } else {
                let name = e.file_name().to_string_lossy().into_owned();
                if widgets.iter().any(|w| name == *w) {
                    if let Ok(s) = fs::read_to_string(&p) {
                        let first = s.lines().next().unwrap_or("");
                        emit(&format!("dapm {name}: {first}"));
                    }
                }
            }
        }
    }
    let base = std::path::Path::new("/sys/kernel/debug/asoc");
    if base.exists() {
        emit("EP-path DAPM widget power:");
        walk(base, 0, emit, WIDGETS);
    } else {
        emit("no /sys/kernel/debug/asoc (ASoC debugfs absent)");
    }
}

/// Earphone analog-path variants: the base is the bone-conduction prime; each variant
/// overrides a few controls to test the TWL6040 earphone power/routing hypotheses.
fn analog_variants() -> Vec<(&'static str, Vec<(&'static str, i32)>)> {
    vec![
        ("V1 current (PLL=HP, PowerMode=HP, HS-L/R=0)", vec![]),
        (
            "V2 stock power modes (PLL=Low-Power, TWL6040 Power Mode=Low-Power)",
            vec![("PLL Selection", 0), ("TWL6040 Power Mode", 0)],
        ),
        (
            "V3 keep Left HSDAC via Headset-Left on (HS-L=1, HS-R=1)",
            vec![("Headset Left Playback", 1), ("Headset Right Playback", 1)],
        ),
    ]
}

/// Plays a tone on each earphone variant, dumping the TWL6040 registers + EP-path DAPM
/// *during* playback (a big buffer keeps the stream RUNNING across the read). The owner
/// confirms which variant (if any) finally makes sound; the register/DAPM dumps are read
/// back over adb to compare the analog state.
pub fn analog_diag(emit: &mut dyn FnMut(&str)) {
    use std::time::Duration;
    let mixer = match Mixer::open() {
        Ok(m) => m,
        Err(e) => {
            emit(&format!("mixer open FAILED: {e}"));
            return;
        }
    };
    let ch = CHANNELS as usize;
    for (idx, (label, overrides)) in analog_variants().into_iter().enumerate() {
        // Countable patterns so the owner can say WHICH variant sounded: V1 = one long
        // tone, V2 = two short beeps, V3 = three short beeps.
        let buf = match idx {
            0 => tone(1000.0, 2000.0, 0.4),
            n => beeps(n + 1, 1000.0, 350.0, 300.0, 0.4),
        };
        emit(&format!(
            "######## ANALOG VARIANT {label} ({} beep(s)) ########",
            if idx == 0 { 1 } else { idx + 1 }
        ));
        // Base bone-conduction prime, then the variant overrides.
        for (name, val, r) in set_prime(&mixer, false) {
            if let Err(e) = r {
                emit(&format!("  set {name}={val}: {e}"));
            }
        }
        for (name, val) in &overrides {
            match mixer.set(name, *val) {
                Ok(()) => emit(&format!(
                    "  override {name} = {val} (readback {:?})",
                    mixer.get(name)
                )),
                Err(e) => emit(&format!("  override {name} = {val}: FAIL: {e}")),
            }
        }
        // Fresh PCM per variant so the DPCM/codec bring-up re-runs with these power modes.
        let pcm = match Pcm::open_params(14784, 2, 0) {
            Ok(p) => p,
            Err(e) => {
                emit(&format!("  pcm open FAILED: {e}"));
                continue;
            }
        };
        let total = buf.len() / ch;
        let one = (pcm.period_size as usize).min(total);
        let play = (|| -> io::Result<()> {
            pcm.prepare()?;
            pcm.writei(&buf[..one * ch])?;
            pcm.start()?;
            Ok(())
        })();
        if let Err(e) = play {
            emit(&format!("  start FAILED: {e}"));
            continue;
        }
        // Stream is RUNNING (playing the first ~300 ms period): read the analog state now.
        dump_codec_state(emit);
        // Finish the tone so the owner can listen.
        if one < total {
            if let Err(e) = pcm.writei(&buf[one * ch..]) {
                emit(&format!("  writei rest FAILED: {e}"));
            }
        }
        let _ = pcm.drain();
        emit(&format!("  {label}: tone played -- ASCOLTA"));
        drop(pcm);
        std::thread::sleep(Duration::from_millis(800));
    }
    emit("ANALOG DIAG done: report which VARIANT made sound; compare twl6040 registers (EARCTL/HSLCTL/HSRCTL/NCPCTRL/LDOCTL/HPPLLCTL) and dapm HSDAC/Earphone Driver On/Off across variants.");
}

// ---- PCM DEVICE sweep (which front-end actually reaches the earphone) ---------------

/// The card exposes three earphone-capable playback front-ends:
///   D0 = "Multimedia"      -> OMAP ABE MultiMedia1 (through the AESS)  [what we use]
///   D6 = "Multimedia"      -> OMAP ABE MultiMedia1 **LP** (low-power ABE path)
///   D8 = "Headset Playback"-> twl6040-legacy (McPDM **direct**, bypasses the ABE/AESS)
/// Every machine-observable state (codec regs, ABE ports, OPP, mixer controls) is identical
/// between our D0 playback on the silent Model B and on the working stock kernel — so the
/// difference must be *which front-end actually carries audio to the earphone*. This plays
/// the SAME 3 s tone on each device in turn (same EP codec route) so the owner reports which
/// device number is audible. The audible one is the front-end the fix must use.
pub fn device_sweep(emit: &mut dyn FnMut(&str)) {
    use std::time::Duration;
    let devs: &[(u32, &str)] = &[
        (0, "ABE MultiMedia1 (current)"),
        (6, "ABE MultiMedia1 LP"),
        (8, "twl6040-legacy / McPDM direct (no ABE)"),
    ];
    let mixer = match Mixer::open() {
        Ok(m) => m,
        Err(e) => {
            emit(&format!("mixer open FAILED: {e}"));
            return;
        }
    };
    for (dev, label) in devs {
        emit(&format!("######## DEVICE {dev}: {label} ########"));
        // (Re)apply the bone-conduction (EP) codec route + ABE DL1 prime before each open.
        for (name, val, r) in set_prime(&mixer, false) {
            if let Err(e) = r {
                emit(&format!("  prime {name}={val}: {e}"));
            }
        }
        let pcm = match Pcm::open_params_dev(*dev, UI_PERIOD, UI_PERIODS, 0) {
            Ok(p) => {
                emit(&format!(
                    "  open pcmC0D{dev}p ok (period {} buffer {} @ {RATE} Hz S16_LE stereo)",
                    p.period_size, p.buffer_size
                ));
                p
            }
            Err(e) => {
                emit(&format!(
                    "  open pcmC0D{dev}p FAILED: {e}{}",
                    errno_suffix(&e)
                ));
                continue;
            }
        };
        emit(&format!(
            ">>> playing 3 s 1 kHz tone on DEVICE {dev} ({label}) -- ASCOLTA ORA"
        ));
        let t0 = std::time::Instant::now();
        match pcm.play_explicit(&tone(1000.0, 3000.0, 0.4)) {
            Ok(()) => emit(&format!(
                "  DEVICE {dev}: played (drained in {} ms)",
                t0.elapsed().as_millis()
            )),
            Err(e) => emit(&format!(
                "  DEVICE {dev}: play FAILED: {e}{}",
                errno_suffix(&e)
            )),
        }
        drop(pcm);
        std::thread::sleep(Duration::from_millis(1500));
    }
    emit("DEVICE SWEEP done: report which DEVICE number (0 / 6 / 8) made audible sound.");
}

// ---- legacy McPDM-direct path (bypasses the ABE/AESS entirely) ----------------------

impl Pcm {
    /// Opens an arbitrary device with an explicit format/rate/channels (for the legacy
    /// McPDM path, which is S32_LE @ 88.2/96 kHz). Leaves the stream PREPARED.
    pub fn open_ex(
        device: u32,
        rate: u32,
        format: u32,
        channels: u32,
        period: u32,
        periods: u32,
    ) -> io::Result<Pcm> {
        let fd = open_snd(&format!("pcmC{CARD}D{device}p"))?;
        let (period_size, buffer_size) =
            set_hw_params_ex(&fd, format, rate, channels, period, periods)?;
        set_sw_params_st(&fd, period_size, buffer_size, buffer_size)?;
        ioctl_ptr(fd.0, SNDRV_PCM_IOCTL_PREPARE, std::ptr::null_mut())
            .map_err(|e| Error::new(e.kind(), format!("PREPARE: {e}")))?;
        Ok(Pcm {
            fd,
            period_size,
            buffer_size,
        })
    }

    /// Raw interleaved byte write; `frames` is the frame count (caller knows bytes/frame).
    pub fn writei_bytes(&self, bytes: &[u8], frames: u32) -> io::Result<usize> {
        let mut x = Xferi {
            result: 0,
            buf: bytes.as_ptr() as *const libc::c_void,
            frames: frames as libc::c_ulong,
        };
        ioctl_ptr(
            self.fd.0,
            SNDRV_PCM_IOCTL_WRITEI_FRAMES,
            &mut x as *mut _ as *mut _,
        )
        .map_err(|e| Error::new(e.kind(), format!("WRITEI: {e}")))?;
        Ok(x.result as usize)
    }
}

/// S32_LE interleaved-stereo sine tone (for the legacy path) at `rate` Hz.
fn tone_s32(freq: f32, ms: f32, peak: f32, rate: u32) -> Vec<u8> {
    let total = (ms * rate as f32 / 1000.0) as usize;
    let edge = (rate as f32 * 0.008) as usize;
    let mut out = Vec::with_capacity(total * 2 * 4);
    let mut phase = 0.0f32;
    for n in 0..total {
        phase += 2.0 * std::f32::consts::PI * freq / rate as f32;
        let env = if n < edge {
            n as f32 / edge as f32
        } else if n >= total - edge {
            (total - n) as f32 / edge as f32
        } else {
            1.0
        };
        let s = (phase.sin() * env * peak * 2_147_483_000.0) as i32;
        let b = s.to_le_bytes();
        out.extend_from_slice(&b);
        out.extend_from_slice(&b);
    }
    out
}

/// `--audio-legacy`: play a tone through the **legacy McPDM-direct** front-end (card0 D8,
/// "Headset Playback" / twl6040-legacy), which bypasses the OMAP4 ABE/AESS ENTIRELY —
/// McPDM reads straight from DMA and drives the TWL6040. This is the decisive "is the ABE
/// the culprit?" test: if this is audible but the ABE path (D0) is not, the AESS data path
/// is at fault; if this is also silent, the codec/McPDM/physical output is. Requires
/// S32_LE at 88.2/96 kHz (OMAP_MCPDM_RATES × TWL6040_FORMATS).
pub fn legacy_play(emit: &mut dyn FnMut(&str)) {
    let mixer = match Mixer::open() {
        Ok(m) => m,
        Err(e) => {
            emit(&format!("mixer open FAILED: {e}"));
            return;
        }
    };
    // EP codec route (ABE DL1 controls are harmless/ignored on the legacy path).
    for (name, val, r) in set_prime(&mixer, false) {
        if let Err(e) = r {
            emit(&format!("  prime {name}={val}: {e}"));
        }
    }
    let rate = 96_000u32;
    // Try a few periods; the McPDM DMA has its own alignment needs.
    let mut pcm = None;
    for (ps, pc) in [(1024u32, 4u32), (960, 4), (1920, 2), (0, 0)] {
        match Pcm::open_ex(8, rate, SNDRV_PCM_FORMAT_S32_LE, 2, ps, pc) {
            Ok(p) => {
                emit(&format!(
                    "  open pcmC0D8p ok: S32_LE {rate} Hz stereo, period {} buffer {} (req {ps}/{pc})",
                    p.period_size, p.buffer_size
                ));
                pcm = Some(p);
                break;
            }
            Err(e) => emit(&format!(
                "  open D8 S32/{rate} period {ps}/{pc}: {e}{}",
                errno_suffix(&e)
            )),
        }
    }
    let Some(pcm) = pcm else {
        emit("  could not open the legacy McPDM path (D8) in any config");
        return;
    };
    let buf = tone_s32(1000.0, 4000.0, 0.4, rate);
    let frames = (buf.len() / 8) as u32; // 8 bytes/frame (S32 stereo)
    let one = pcm.period_size.min(frames);
    emit(">>> playing 4 s 1 kHz tone on the LEGACY McPDM path (no ABE) -- ASCOLTA ORA");
    let r = (|| -> io::Result<()> {
        pcm.prepare()?;
        pcm.writei_bytes(&buf[..(one as usize) * 8], one)?;
        pcm.start()?;
        if one < frames {
            pcm.writei_bytes(&buf[(one as usize) * 8..], frames - one)?;
        }
        pcm.drain()
    })();
    match r {
        Ok(()) => emit("  legacy path: tone played/drained OK"),
        Err(e) => emit(&format!(
            "  legacy path: play FAILED: {e}{}",
            errno_suffix(&e)
        )),
    }
}

// ---- stock-HAL-matched playback: MMAP + NO_PERIOD_WAKEUP (NOIRQ) --------------------
//
// The stock audio.primary.glass_1 HAL opens the earphone playback PCM with
// pcm_open(card=0, device=0, flags = PCM_MMAP | PCM_NOIRQ, config{ABE period}), i.e. it
// drives the OMAP4 ABE front-end in MMAP + no-period-wakeup mode and feeds it by writing
// into the mmap'd DMA buffer and advancing appl_ptr via SYNC_PTR — NOT with blocking
// writei() as our launcher does. This reproduces that exact path, the one real difference
// found between the (audible) stock HAL and our (silent) launcher on the identical kernel.

/// hw_params for the MMAP/NOIRQ path: MMAP_INTERLEAVED access + NO_PERIOD_WAKEUP flag.
fn set_hw_params_mmap(fd: &Fd, period_size: u32, periods: u32) -> io::Result<(u32, u32)> {
    let mut p = [0u8; SIZEOF_HW_PARAMS];
    put_u32(&mut p, 0, SNDRV_PCM_HW_PARAMS_NO_PERIOD_WAKEUP); // flags @ offset 0
    for k in 0..3 {
        put_u32(&mut p, HWP_MASK0 + k * 32, 0xffff_ffff);
        put_u32(&mut p, HWP_MASK0 + k * 32 + 4, 0xffff_ffff);
    }
    for i in 0..12 {
        put_u32(&mut p, HWP_INTERVAL0 + i * 12, 0);
        put_u32(&mut p, HWP_INTERVAL0 + i * 12 + 4, 0xffff_ffff);
    }
    put_u32(&mut p, HWP_RMASK, 0xffff_ffff);
    put_u32(&mut p, HWP_INFO, 0xffff_ffff);
    set_mask(&mut p, P_ACCESS, SNDRV_PCM_ACCESS_MMAP_INTERLEAVED);
    set_mask(&mut p, P_FORMAT, SNDRV_PCM_FORMAT_S16_LE);
    set_interval_exact(&mut p, P_CHANNELS, CHANNELS);
    set_interval_exact(&mut p, P_RATE, RATE);
    if period_size > 0 {
        set_interval_exact(&mut p, P_PERIOD_SIZE, period_size);
    }
    if periods > 0 {
        set_interval_exact(&mut p, P_PERIODS, periods);
    }
    ioctl_ptr(fd.0, SNDRV_PCM_IOCTL_HW_PARAMS, p.as_mut_ptr() as *mut _)?;
    let got_period = get_u32(&p, HWP_INTERVAL0 + (P_PERIOD_SIZE - FIRST_INTERVAL) * 12);
    let got_buffer = get_u32(&p, HWP_INTERVAL0 + (P_BUFFER_SIZE - FIRST_INTERVAL) * 12);
    let period = if got_period == 0 || got_period == 0xffff_ffff {
        period_size.max(1024)
    } else {
        got_period
    };
    let buffer = if got_buffer == 0 || got_buffer == 0xffff_ffff {
        period * periods.max(2)
    } else {
        got_buffer
    };
    Ok((period, buffer))
}

/// SYNC_PTR: push appl_ptr + avail_min to the kernel and read back hw_ptr (frames). HWSYNC
/// forces a DMA-position resync. appl_ptr/hw_ptr are monotonic frame counters.
fn sync_ptr(fd: RawFd, hwsync: bool, appl: u32, avail_min: u32) -> io::Result<u32> {
    let mut sp = [0u8; SIZEOF_SYNC_PTR];
    // HWSYNC forces a DMA-position re-read; only valid once the stream is RUNNING.
    put_u32(
        &mut sp,
        SP_FLAGS,
        if hwsync { SNDRV_PCM_SYNC_PTR_HWSYNC } else { 0 },
    );
    put_u32(&mut sp, SP_CTRL_APPLPTR, appl);
    put_u32(&mut sp, SP_CTRL_AVAILMIN, avail_min);
    ioctl_ptr(fd, SNDRV_PCM_IOCTL_SYNC_PTR, sp.as_mut_ptr() as *mut _)
        .map_err(|e| Error::new(e.kind(), format!("SYNC_PTR: {e}")))?;
    Ok(get_u32(&sp, SP_STATUS_HWPTR))
}

/// Plays `samples` (S16 interleaved stereo) on card0 D`device` via MMAP + NO_PERIOD_WAKEUP,
/// exactly like the stock HAL. Returns the drain time in ms.
pub fn mmap_play(
    emit: &mut dyn FnMut(&str),
    device: u32,
    period: u32,
    periods: u32,
    start_threshold: u32,
    samples: &[i16],
) -> io::Result<u128> {
    use std::time::Instant;
    let ch = CHANNELS as usize;
    let fd = open_snd(&format!("pcmC{CARD}D{device}p"))?;
    let (period_size, buffer_size) = set_hw_params_mmap(&fd, period, periods)?;
    let st = if start_threshold == 0 {
        buffer_size
    } else {
        start_threshold
    };
    set_sw_params_st(&fd, period_size, buffer_size, st)?;
    ioctl_ptr(fd.0, SNDRV_PCM_IOCTL_PREPARE, std::ptr::null_mut())
        .map_err(|e| Error::new(e.kind(), format!("PREPARE: {e}")))?;
    emit(&format!(
        "mmap: pcmC0D{device}p period {period_size} buffer {buffer_size} frames, MMAP+NOIRQ"
    ));

    // mmap the DMA data buffer (offset 0 = DATA).
    let map_len = buffer_size as usize * ch * 2; // 2 bytes/sample (S16)
    let ptr = unsafe {
        libc::mmap(
            std::ptr::null_mut(),
            map_len,
            libc::PROT_READ | libc::PROT_WRITE,
            libc::MAP_SHARED,
            fd.0,
            0,
        )
    };
    if ptr == libc::MAP_FAILED {
        return Err(Error::new(
            Error::last_os_error().kind(),
            format!("mmap data buffer: {}", Error::last_os_error()),
        ));
    }
    let dma: &mut [i16] =
        unsafe { std::slice::from_raw_parts_mut(ptr as *mut i16, buffer_size as usize * ch) };

    let total = samples.len() / ch; // total frames to play
    let buff = buffer_size as usize;
    let copy_frame = |dma: &mut [i16], dst_frame: usize, src_frame: usize, samples: &[i16]| {
        let d = (dst_frame % buff) * ch;
        let s = src_frame * ch;
        dma[d] = samples[s];
        dma[d + 1] = samples[s + 1];
    };

    // Prime: fill one full buffer (or all we have), set appl_ptr, START.
    let prime = buff.min(total);
    for f in 0..prime {
        copy_frame(dma, f, f, samples);
    }
    let mut appl: u32 = prime as u32;
    let mut src: usize = prime;
    emit(&format!(
        "mmap: primed {prime} frames; setting appl_ptr pre-START"
    ));
    sync_ptr(fd.0, false, appl, period_size)
        .map_err(|e| Error::new(e.kind(), format!("pre-START {e}")))?;
    emit("mmap: appl set; issuing START");
    ioctl_ptr(fd.0, SNDRV_PCM_IOCTL_START, std::ptr::null_mut())
        .map_err(|e| Error::new(e.kind(), format!("START: {e}")))?;
    emit("mmap: START ok; entering fill loop");

    let t0 = Instant::now();
    let mut spins = 0u32;
    while src < total {
        // Refresh the DMA position: the dedicated HWSYNC ioctl, then read hw_ptr.
        let _ = ioctl_ptr(fd.0, SNDRV_PCM_IOCTL_HWSYNC, std::ptr::null_mut());
        let hw = sync_ptr(fd.0, false, appl, period_size)?;
        // free space = buffer - (appl - hw)
        let in_flight = appl.wrapping_sub(hw);
        let free = buffer_size.saturating_sub(in_flight);
        if free < period_size {
            // no room yet: the ABE is draining; wait ~ one period then retry.
            unsafe { libc::usleep(2000) };
            spins += 1;
            if spins > 3000 {
                return Err(Error::other(format!(
                    "mmap: stalled (hw_ptr={hw} appl={appl} not advancing)"
                )));
            }
            continue;
        }
        spins = 0;
        let n = (free as usize).min(total - src);
        for i in 0..n {
            copy_frame(dma, (appl as usize) + i, src + i, samples);
        }
        appl = appl.wrapping_add(n as u32);
        src += n;
        sync_ptr(fd.0, false, appl, period_size)?;
    }
    // wait for the queued tail to drain
    let mut dspins = 0u32;
    loop {
        let _ = ioctl_ptr(fd.0, SNDRV_PCM_IOCTL_HWSYNC, std::ptr::null_mut());
        let hw = sync_ptr(fd.0, false, appl, period_size)?;
        if appl.wrapping_sub(hw) == 0 {
            break;
        }
        unsafe { libc::usleep(5000) };
        dspins += 1;
        if dspins > 400 {
            break;
        }
    }
    let ms = t0.elapsed().as_millis();
    let _ = ioctl_ptr(fd.0, SNDRV_PCM_IOCTL_DROP, std::ptr::null_mut());
    unsafe { libc::munmap(ptr, map_len) };
    Ok(ms)
}

/// `--audio-mmap`: the stock-HAL-matched earphone playback (MMAP + NOIRQ on the ABE D0).
pub fn mmap_diag(emit: &mut dyn FnMut(&str)) {
    let mixer = match Mixer::open() {
        Ok(m) => m,
        Err(e) => {
            emit(&format!("mixer open FAILED: {e}"));
            return;
        }
    };
    for (name, val, r) in set_prime(&mixer, false) {
        if let Err(e) = r {
            emit(&format!("  prime {name}={val}: {e}"));
        }
    }
    let _ = mixer.set("Earphone Playback Volume", 12);
    emit(">>> MMAP+NOIRQ earphone tone (stock-HAL path): 6 s 1 kHz -- ASCOLTA ORA");
    // ABE period like the stock HAL; fall back to the proven 1056/4 if the big one is rejected.
    let buf = tone(1000.0, 6000.0, 0.4);
    let mut done = false;
    for (ps, pc) in [(1056u32, 4u32), (1920, 4), (14784, 2)] {
        match mmap_play(emit, 0, ps, pc, 0, &buf) {
            Ok(ms) => {
                emit(&format!(
                    "  MMAP play OK on period {ps}/{pc}, drained in {ms} ms"
                ));
                done = true;
                break;
            }
            Err(e) => emit(&format!(
                "  MMAP period {ps}/{pc} FAILED: {e}{}",
                errno_suffix(&e)
            )),
        }
    }
    if !done {
        emit("  MMAP path failed on all periods");
    }
}

/// `--audio-stock-exact`: reproduce the stock HAL's earphone pcm_open BYTE-FOR-BYTE —
/// MMAP + NO_PERIOD_WAKEUP, card 0 device 0, **period 14784, start_threshold 2112**,
/// S16_LE / 48 kHz stereo (the exact values disassembled from audio.primary.glass_1.so),
/// on the bone-conduction EP route. The earlier --audio-mmap used period 1056/4, not the
/// stock 14784; this is the true byte-for-byte branch-B test.
pub fn stock_exact_diag(emit: &mut dyn FnMut(&str)) {
    let mixer = match Mixer::open() {
        Ok(m) => m,
        Err(e) => {
            emit(&format!("mixer open FAILED: {e}"));
            return;
        }
    };
    for (name, val, r) in set_prime(&mixer, false) {
        if let Err(e) = r {
            emit(&format!("  prime {name}={val}: {e}"));
        }
    }
    let _ = mixer.set("Earphone Playback Volume", 12);
    emit(">>> STOCK-EXACT earphone tone: MMAP+NOIRQ, period 14784, start_threshold 2112, D0 EP -- ASCOLTA");
    let buf = tone(1000.0, 6000.0, 0.4);
    match mmap_play(emit, 0, 14784, 2, 2112, &buf) {
        Ok(ms) => emit(&format!("  stock-exact MMAP play OK, drained in {ms} ms")),
        Err(e) => {
            emit(&format!(
                "  stock-exact (14784/2) FAILED: {e}{}",
                errno_suffix(&e)
            ));
            // fall back to period 14784 x3 (buffer ~44352) in case /2 is rejected
            match mmap_play(emit, 0, 14784, 3, 2112, &buf) {
                Ok(ms) => emit(&format!("  stock-exact (14784/3) OK, drained in {ms} ms")),
                Err(e) => emit(&format!(
                    "  stock-exact (14784/3) FAILED: {e}{}",
                    errno_suffix(&e)
                )),
            }
        }
    }
}

/// `--audio-writei14784`: EP tone via plain blocking writei() at the STOCK ABE period
/// (14784 / start_threshold 2112), NOT mmap. Isolates the variable: if this is audible but
/// writei@1056 is not, the key is the ABE period, not MMAP; if only --audio-stock-exact
/// (MMAP) is audible, the key is MMAP+NOIRQ.
pub fn writei_stock_diag(emit: &mut dyn FnMut(&str)) {
    let mixer = match Mixer::open() {
        Ok(m) => m,
        Err(e) => {
            emit(&format!("mixer open FAILED: {e}"));
            return;
        }
    };
    for (name, val, r) in set_prime(&mixer, false) {
        if let Err(e) = r {
            emit(&format!("  prime {name}={val}: {e}"));
        }
    }
    let _ = mixer.set("Earphone Playback Volume", 12);
    let pcm = match Pcm::open_params(14784, 2, 2112) {
        Ok(p) => {
            emit(&format!(
                "  writei pcmC0D0p period {} buffer {} (stock 14784/st2112), EP route",
                p.period_size, p.buffer_size
            ));
            p
        }
        Err(e) => {
            emit(&format!("  open 14784/2 FAILED: {e}{}", errno_suffix(&e)));
            return;
        }
    };
    emit(">>> WRITEI @ stock period 14784 (bone conduction) 6 s 1 kHz -- ASCOLTA");
    match pcm.play_explicit(&tone(1000.0, 6000.0, 0.4)) {
        Ok(()) => emit("  writei@14784: played/drained OK"),
        Err(e) => emit(&format!("  writei@14784 FAILED: {e}{}", errno_suffix(&e))),
    }
}

/// Reads the REAL TWL6040 registers via the regmap debugfs (works while the codec is
/// powered, i.e. during playback) — unlike the I2C path which EBUSYs under the driver.
fn dump_regmap_debugfs(emit: &mut dyn FnMut(&str)) {
    let Ok(rd) = fs::read_dir("/sys/kernel/debug/regmap") else {
        emit("regmap: /sys/kernel/debug/regmap absent");
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        let name = fs::read_to_string(p.join("name")).unwrap_or_default();
        if name.trim() == "twl6040" {
            match fs::read_to_string(p.join("registers")) {
                Ok(s) => emit(&format!(
                    "regmap twl6040 registers: {}",
                    s.replace('\n', " ")
                )),
                Err(err) => emit(&format!("regmap registers read FAIL: {err}")),
            }
            return;
        }
    }
    emit("regmap: no twl6040 node");
}

/// `--audio-dump-state`: dump the FULL live audio state WITHOUT playing anything — real
/// TWL6040 regs (regmap debugfs), ABE ports + OPP, every EP/DL1 mixer control value, and
/// the EP DAPM widgets + codec_reg cache. Run this WHILE the stock HAL is playing audible
/// audio to capture the gold reference, then run it during our launcher's playback and diff.
pub fn dump_state(emit: &mut dyn FnMut(&str)) {
    emit("==== LIVE AUDIO STATE DUMP (no playback) ====");
    dump_regmap_debugfs(emit);
    // ABE ports + OPP
    let rd = |f: &str| fs::read_to_string(f).unwrap_or_default().trim().to_string();
    emit(&format!(
        "abe opp_level={} mm_dl1={}/{} mm_dl_lp={}/{} pdmdl1={}/{} pdmdl2={}/{}",
        rd("/sys/kernel/debug/omap-abe/opp_level"),
        rd("/sys/kernel/debug/abe_port/mm_dl1_state"),
        rd("/sys/kernel/debug/abe_port/mm_dl1_users"),
        rd("/sys/kernel/debug/abe_port/mm_dl_lp_state"),
        rd("/sys/kernel/debug/abe_port/mm_dl_lp_users"),
        rd("/sys/kernel/debug/abe_port/pdmdl1_state"),
        rd("/sys/kernel/debug/abe_port/pdmdl1_users"),
        rd("/sys/kernel/debug/abe_port/pdmdl2_state"),
        rd("/sys/kernel/debug/abe_port/pdmdl2_users"),
    ));
    // Mixer control values
    match Mixer::open() {
        Ok(mixer) => {
            for line in ep_state_report(&mixer) {
                emit(&line);
            }
        }
        Err(e) => emit(&format!("mixer open FAIL: {e}")),
    }
    dump_dapm(emit);
    dump_codec_reg_debugfs(emit);
    emit("==== END STATE DUMP ====");
}

/// `--audio-headset`: play the tone on the HEADSET / USB-earbud output (TWL6040 HSOL/HSOR)
/// instead of the bone-conduction earpiece — Headset L/R on, Earphone off. The headset
/// output is far louder/clearer and bypasses the bone-conduction transducer, so it
/// disambiguates a weak/faulty EP transducer from a software fault: if the owner (earbud
/// plugged) hears THIS but not the EP tone, the problem is EP/bone-conduction-specific.
pub fn headset_diag(emit: &mut dyn FnMut(&str)) {
    let mixer = match Mixer::open() {
        Ok(m) => m,
        Err(e) => {
            emit(&format!("mixer open FAILED: {e}"));
            return;
        }
    };
    emit(&format!("headset sysfs detected: {}", headset_present()));
    // Prime the HEADSET route (headset=true: HSOL/HSOR on, Earphone off).
    for (name, val, r) in set_prime(&mixer, true) {
        match r {
            Ok(()) => emit(&format!("  mixer {name}={val}: ok")),
            Err(e) => emit(&format!("  mixer {name}={val}: {e}")),
        }
    }
    let _ = mixer.set("Headset Playback Volume", 13);
    let pcm = match Pcm::open_params(UI_PERIOD, UI_PERIODS, 0) {
        Ok(p) => {
            emit(&format!(
                "  pcmC0D0p open ok (period {} buffer {})",
                p.period_size, p.buffer_size
            ));
            p
        }
        Err(e) => {
            emit(&format!("  pcm open FAILED: {e}"));
            return;
        }
    };
    emit(">>> HEADSET tone (plug a USB earbud): 6 s 1 kHz on HSOL/HSOR -- ASCOLTA");
    match pcm.play_explicit(&tone(1000.0, 6000.0, 0.4)) {
        Ok(()) => emit("  headset tone: played/drained OK"),
        Err(e) => emit(&format!("  headset tone FAILED: {e}{}", errno_suffix(&e))),
    }
}
