//! Commands from the Java side (UI thread) to the render thread, and the pipe that wakes the
//! render thread out of its `poll` when one arrives. Platform independent and host-tested.

use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Duration;

/// A native window handle passed between threads (an `ANativeWindow*` with one reference
/// held for the render thread).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowPtr(pub *mut std::ffi::c_void);

// SAFETY: an ANativeWindow is reference counted and usable from any thread; the render
// thread is the only user once the pointer is handed over.
unsafe impl Send for WindowPtr {}

/// What the activity tells the render thread.
#[derive(Debug)]
pub enum Command {
    /// A new surface; the render thread owns the window reference from now on.
    SurfaceCreated(WindowPtr),
    SurfaceChanged {
        width: i32,
        height: i32,
    },
    /// The surface is going away: release it, then answer on the channel. The Java side
    /// waits for the answer before it lets the surface go.
    SurfaceDestroyed(Sender<()>),
    Resume,
    Pause,
    /// A platform event line (`glass_launcher::platform::PlatformEvent`).
    Event(String),
    Quit,
}

/// The write end of the wake pipe plus the command channel: cloneable, used from the JNI
/// threads.
#[derive(Clone)]
pub struct Control {
    tx: Sender<Command>,
    wake: std::sync::Arc<OwnedFd>,
}

/// The render thread's side: the commands and the read end of the pipe.
pub struct Inbox {
    rx: Receiver<Command>,
    wake: OwnedFd,
}

/// A channel with a self-pipe, so the receiver can `poll` on [`Inbox::fd`] together with its
/// sockets.
pub fn channel() -> io::Result<(Control, Inbox)> {
    let mut fds = [0 as libc::c_int; 2];
    // SAFETY: `fds` has room for the two descriptors.
    if unsafe { libc::pipe(fds.as_mut_ptr()) } != 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: both descriptors were just created and are owned here.
    let (r, w) = unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) };
    for fd in [r.as_raw_fd(), w.as_raw_fd()] {
        // SAFETY: plain fcntl calls on our own descriptors.
        unsafe {
            let fl = libc::fcntl(fd, libc::F_GETFL);
            libc::fcntl(fd, libc::F_SETFL, fl | libc::O_NONBLOCK);
            libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC);
        }
    }
    let (tx, rx) = mpsc::channel();
    Ok((
        Control {
            tx,
            wake: std::sync::Arc::new(w),
        },
        Inbox { rx, wake: r },
    ))
}

impl Control {
    /// Queues `c` and wakes the render thread. False if the render thread is gone.
    pub fn send(&self, c: Command) -> bool {
        if self.tx.send(c).is_err() {
            return false;
        }
        let b = 1u8;
        // SAFETY: one byte from a valid buffer; a full pipe (EAGAIN) already means "wake".
        unsafe { libc::write(self.wake.as_raw_fd(), (&b as *const u8).cast(), 1) };
        true
    }

    /// Asks the render thread to release the surface and waits up to `timeout` for it.
    /// False if it did not answer in time (it is stuck or gone).
    pub fn surface_destroyed(&self, timeout: Duration) -> bool {
        let (tx, rx) = mpsc::channel();
        if !self.send(Command::SurfaceDestroyed(tx)) {
            return false;
        }
        rx.recv_timeout(timeout).is_ok()
    }
}

impl Inbox {
    /// The descriptor that becomes readable when commands are queued.
    pub fn fd(&self) -> RawFd {
        self.wake.as_raw_fd()
    }

    /// Every queued command, oldest first; empties the wake pipe.
    pub fn drain(&self) -> Vec<Command> {
        let mut buf = [0u8; 64];
        // SAFETY: reads into a valid buffer from our non-blocking pipe.
        while unsafe { libc::read(self.wake.as_raw_fd(), buf.as_mut_ptr().cast(), buf.len()) } > 0 {
        }
        self.rx.try_iter().collect()
    }
}

/// What the render thread waits on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Woken {
    pub commands: bool,
    pub socket: bool,
}

/// Blocks until the inbox or `socket` is readable (or hung up), or `timeout` passes
/// (`None`: no limit).
pub fn wait(inbox: &Inbox, socket: Option<RawFd>, timeout: Option<Duration>) -> Woken {
    let mut fds = vec![libc::pollfd {
        fd: inbox.fd(),
        events: libc::POLLIN,
        revents: 0,
    }];
    if let Some(s) = socket {
        fds.push(libc::pollfd {
            fd: s,
            events: libc::POLLIN,
            revents: 0,
        });
    }
    let ms = match timeout {
        None => -1,
        Some(d) => d.as_millis().min(i32::MAX as u128) as i32,
    };
    // SAFETY: `fds` is a valid array of pollfd of the given length.
    let n = unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, ms) };
    if n <= 0 {
        return Woken {
            commands: false,
            socket: false,
        };
    }
    let ready = |p: &libc::pollfd| p.revents & (libc::POLLIN | libc::POLLHUP | libc::POLLERR) != 0;
    Woken {
        commands: ready(&fds[0]),
        socket: fds.get(1).is_some_and(ready),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    #[test]
    fn a_command_wakes_the_wait() {
        let (c, inbox) = channel().unwrap();
        let t = Instant::now();
        let w = wait(&inbox, None, Some(Duration::from_millis(30)));
        assert!(!w.commands);
        assert!(t.elapsed() >= Duration::from_millis(25));
        let c2 = c.clone();
        let h = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(20));
            c2.send(Command::Resume);
            c2.send(Command::Event("provisioned\ttrue".into()));
        });
        let w = wait(&inbox, None, None);
        assert!(w.commands);
        h.join().unwrap();
        let cmds = inbox.drain();
        assert!(matches!(cmds[0], Command::Resume));
        assert!(matches!(&cmds[1], Command::Event(e) if e == "provisioned\ttrue"));
        // Drained: no longer readable.
        assert!(!wait(&inbox, None, Some(Duration::ZERO)).commands);
    }

    #[test]
    fn surface_destroyed_waits_for_the_answer() {
        let (c, inbox) = channel().unwrap();
        let h = std::thread::spawn(move || loop {
            wait(&inbox, None, None);
            for cmd in inbox.drain() {
                if let Command::SurfaceDestroyed(ack) = cmd {
                    ack.send(()).unwrap();
                    return;
                }
            }
        });
        assert!(c.surface_destroyed(Duration::from_secs(5)));
        h.join().unwrap();
        // The render thread is gone: no answer.
        assert!(!c.surface_destroyed(Duration::from_millis(10)));
    }

    #[test]
    fn many_commands_do_not_block_the_sender() {
        let (c, inbox) = channel().unwrap();
        for _ in 0..100_000 {
            assert!(c.send(Command::Pause));
        }
        assert_eq!(inbox.drain().len(), 100_000);
    }
}
