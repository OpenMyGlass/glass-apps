//! What the launcher remembers about the phone bond across restarts and reboots: the last bond
//! glassd reported, and a question about a new bond the owner has not answered yet. Without
//! it, a phone that bonded just before a crash or a reboot would be taken as the owner's.
//!
//! One small text file (`phone-bond`) in the launcher's state directory, next to the setup
//! marker on Android. Names are written in hex, so any byte a phone sends is safe. It is
//! replaced atomically: written to a temporary file, synced, then renamed over the old one.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// The file name in the state directory.
pub const FILE: &str = "phone-bond";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BondState {
    /// The last bond glassd reported: (`paired`, `paired_name`).
    pub seen: Option<(bool, String)>,
    /// A bond the owner has not said is theirs: the phone's name.
    pub question: Option<String>,
}

fn hex(s: &str) -> String {
    s.bytes().map(|b| format!("{b:02x}")).collect()
}

fn unhex(s: &str) -> Option<String> {
    if s.len() % 2 != 0 {
        return None;
    }
    let bytes: Option<Vec<u8>> = (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok())
        .collect();
    String::from_utf8(bytes?).ok()
}

impl BondState {
    /// The file's text.
    pub fn encode(&self) -> String {
        let mut out = String::new();
        if let Some((paired, name)) = &self.seen {
            out.push_str(&format!("seen {} {}\n", u8::from(*paired), hex(name)));
        }
        if let Some(name) = &self.question {
            out.push_str(&format!("question {}\n", hex(name)));
        }
        out
    }

    /// Reads the file's text; a line it does not understand is skipped.
    pub fn decode(text: &str) -> BondState {
        let mut s = BondState::default();
        for line in text.lines() {
            let f: Vec<&str> = line.split(' ').collect();
            match f.as_slice() {
                ["seen", p, name] => {
                    if let (Some(p), Some(name)) = (
                        match *p {
                            "1" => Some(true),
                            "0" => Some(false),
                            _ => None,
                        },
                        unhex(name),
                    ) {
                        s.seen = Some((p, name));
                    }
                }
                ["question", name] => s.question = unhex(name),
                _ => {}
            }
        }
        s
    }
}

/// The file in `dir`.
pub fn path_in(dir: &Path) -> PathBuf {
    dir.join(FILE)
}

/// Loads the state; a missing file is the empty state.
pub fn load(path: &Path) -> io::Result<BondState> {
    match fs::read_to_string(path) {
        Ok(text) => Ok(BondState::decode(&text)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(BondState::default()),
        Err(e) => Err(e),
    }
}

/// Replaces the file atomically (temporary file, sync, rename, sync of the directory).
pub fn save(path: &Path, state: &BondState) -> io::Result<()> {
    let dir = path.parent().unwrap_or(Path::new("."));
    fs::create_dir_all(dir)?;
    let tmp = path.with_extension("tmp");
    {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(state.encode().as_bytes())?;
        f.sync_all()?;
    }
    fs::rename(&tmp, path)?;
    // The rename itself is durable once the directory is synced (best effort).
    if let Ok(d) = fs::File::open(dir) {
        let _ = d.sync_all();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_any_name() {
        let s = BondState {
            seen: Some((true, "Pixel 8 \n\t é".into())),
            question: Some("a b".into()),
        };
        assert_eq!(BondState::decode(&s.encode()), s);
        assert_eq!(BondState::decode(""), BondState::default());
        assert_eq!(
            BondState::decode("seen 2 41\nquestion zz\nother\n"),
            BondState::default()
        );
    }

    #[test]
    fn saves_atomically_and_loads() {
        let d = std::env::temp_dir().join(format!("glass-bond-{}", std::process::id()));
        let p = path_in(&d.join("sub"));
        assert_eq!(load(&p).unwrap(), BondState::default());
        let s = BondState {
            seen: Some((false, String::new())),
            question: None,
        };
        save(&p, &s).unwrap();
        assert_eq!(load(&p).unwrap(), s);
        assert!(!p.with_extension("tmp").exists());
        let _ = fs::remove_dir_all(&d);
    }
}
