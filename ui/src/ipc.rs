//! Types generated from proto/ipc.proto and the socket transport.

use std::io::{self, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;

include!(concat!(env!("OUT_DIR"), "/wassup.ipc.v1.rs"));

/// Same limit as the daemon's protodelim default.
const MAX_FRAME: usize = 4 << 20;

pub fn socket_path() -> io::Result<PathBuf> {
    // No fallback to /tmp: it is shared with other users.
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(|dir| PathBuf::from(dir).join("wassup.sock"))
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "XDG_RUNTIME_DIR is not set"))
}

/// A connection to the daemon. Requests are written from the GTK main
/// thread; events are read on a separate thread and handed back over a
/// channel so a blocking read never freezes the UI.
pub struct Connection {
    stream: UnixStream,
    next_id: std::cell::Cell<u64>,
}

impl Connection {
    pub fn open() -> io::Result<(Self, async_channel::Receiver<io::Result<Event>>)> {
        let stream = UnixStream::connect(socket_path()?)?;
        let mut reader = stream.try_clone()?;
        let (tx, rx) = async_channel::unbounded();
        std::thread::spawn(move || {
            loop {
                let event = read_frame::<Event>(&mut reader);
                let failed = event.is_err();
                if tx.send_blocking(event).is_err() || failed {
                    break;
                }
            }
        });
        let conn = Self {
            stream,
            next_id: std::cell::Cell::new(0),
        };
        Ok((conn, rx))
    }

    /// Sends a request and returns the id its reply will carry.
    pub fn send(&self, kind: request::Kind) -> io::Result<u64> {
        let id = self.next_id.get() + 1;
        self.next_id.set(id);
        let req = Request {
            id,
            kind: Some(kind),
        };
        write_frame(&mut &self.stream, &req)?;
        Ok(id)
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        // Unblocks the reader thread so it can exit.
        self.stream.shutdown(std::net::Shutdown::Both).ok();
    }
}

pub fn write_frame(w: &mut impl Write, msg: &impl prost::Message) -> io::Result<()> {
    w.write_all(&msg.encode_length_delimited_to_vec())
}

pub fn read_frame<M: prost::Message + Default>(r: &mut impl Read) -> io::Result<M> {
    // The length prefix is a varint of at most 10 bytes, each but the last with the high bit set.
    let mut prefix = Vec::with_capacity(10);
    loop {
        let mut byte = [0];
        r.read_exact(&mut byte)?;
        prefix.push(byte[0]);
        if byte[0] < 0x80 || prefix.len() == 10 {
            break;
        }
    }
    let len = prost::decode_length_delimiter(&prefix[..]).map_err(invalid)?;
    if len > MAX_FRAME {
        return Err(invalid(format!("frame of {len} bytes exceeds {MAX_FRAME}")));
    }
    let mut buf = vec![0; len];
    r.read_exact(&mut buf)?;
    M::decode(&buf[..]).map_err(invalid)
}

fn invalid(e: impl Into<Box<dyn std::error::Error + Send + Sync>>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, e)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn list_accounts(id: u64) -> Request {
        Request {
            id,
            kind: Some(request::Kind::ListAccounts(ListAccounts {})),
        }
    }

    #[test]
    fn frames_round_trip() {
        let mut buf = Vec::new();
        write_frame(&mut buf, &list_accounts(1)).unwrap();
        write_frame(&mut buf, &list_accounts(300)).unwrap();
        let mut r = Cursor::new(buf);
        assert_eq!(read_frame::<Request>(&mut r).unwrap(), list_accounts(1));
        assert_eq!(read_frame::<Request>(&mut r).unwrap(), list_accounts(300));
        assert_eq!(
            read_frame::<Request>(&mut r).unwrap_err().kind(),
            io::ErrorKind::UnexpectedEof
        );
    }

    #[test]
    fn rejects_oversized_frame() {
        let mut prefix = Vec::new();
        prost::encode_length_delimiter(MAX_FRAME + 1, &mut prefix).unwrap();
        let mut r = Cursor::new(prefix);
        let err = read_frame::<Request>(&mut r).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }

    /// Talks to the real Go daemon. Needs the Go toolchain.
    #[test]
    #[ignore = "builds and runs the Go daemon"]
    fn exchanges_with_daemon() {
        use std::os::unix::fs::DirBuilderExt;
        use std::process::Command;
        use std::time::{Duration, Instant};

        let dir = std::env::temp_dir().join(format!("wassup-test-{}", std::process::id()));
        std::fs::DirBuilder::new().mode(0o700).create(&dir).unwrap();
        let bin = dir.join("daemon");
        let built = Command::new("go")
            .args(["build", "-o"])
            .arg(&bin)
            .current_dir("../daemon")
            .status()
            .unwrap();
        assert!(built.success());
        let mut daemon = Command::new(&bin)
            .env("XDG_RUNTIME_DIR", &dir)
            .env("XDG_DATA_HOME", &dir)
            .env("XDG_CACHE_HOME", &dir)
            .spawn()
            .unwrap();

        let sock = dir.join("wassup.sock");
        let deadline = Instant::now() + Duration::from_secs(5);
        let stream = loop {
            match UnixStream::connect(&sock) {
                Ok(s) => break Ok(s),
                Err(e) if Instant::now() > deadline => break Err(e),
                Err(_) => std::thread::sleep(Duration::from_millis(20)),
            }
        };
        let result = stream.and_then(|mut s| {
            write_frame(&mut s, &list_accounts(42))?;
            read_frame::<Event>(&mut s)
        });
        daemon.kill().ok();
        daemon.wait().ok();
        std::fs::remove_dir_all(&dir).ok();

        let event = result.unwrap();
        assert_eq!(event.request_id, 42);
        assert!(matches!(event.kind, Some(event::Kind::Accounts(_))));
    }
}
