#[cfg(target_os = "linux")]
use std::ffi::OsString;
use std::path::PathBuf;

/// A Unix domain socket transport in a D-Bus address.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unix {
    path: UnixSocket,
}

impl Unix {
    /// Create a new Unix transport with the given path.
    pub fn new(path: UnixSocket) -> Self {
        Self { path }
    }

    /// The path.
    pub fn path(&self) -> &UnixSocket {
        &self.path
    }

    /// Take the path, consuming `self`.
    pub fn take_path(self) -> UnixSocket {
        self.path
    }

    pub(super) fn from_options(opts: std::collections::HashMap<&str, &str>) -> crate::Result<Self> {
        let path = opts.get("path");
        let abs = opts.get("abstract");
        let dir = opts.get("dir");
        let tmpdir = opts.get("tmpdir");
        let path = match (path, abs, dir, tmpdir) {
            (Some(p), None, None, None) => UnixSocket::File(PathBuf::from(p)),
            #[cfg(target_os = "linux")]
            (None, Some(p), None, None) => UnixSocket::Abstract(OsString::from(p)),
            #[cfg(not(target_os = "linux"))]
            (None, Some(_), None, None) => {
                return Err(crate::Error::Address(
                    "abstract sockets currently Linux-only".to_owned(),
                ));
            }
            (None, None, Some(p), None) => UnixSocket::Dir(PathBuf::from(p)),
            (None, None, None, Some(p)) => UnixSocket::TmpDir(PathBuf::from(p)),
            _ => {
                return Err(crate::Error::Address("unix: address is invalid".to_owned()));
            }
        };

        Ok(Self::new(path))
    }
}

/// A Unix domain socket path in a D-Bus address.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum UnixSocket {
    /// A path to a unix domain socket on the filesystem.
    File(PathBuf),
    /// A abstract unix domain socket name.
    #[cfg(target_os = "linux")]
    Abstract(OsString),
    /// A listenable address using the specified path, in which a socket file with a random file
    /// name starting with 'dbus-' will be created by the server. See [UNIX domain socket address]
    /// reference documentation.
    ///
    /// This address is mostly relevant to server (typically bus broker) implementations.
    ///
    /// [UNIX domain socket address]: https://dbus.freedesktop.org/doc/dbus-specification.html#transports-unix-domain-sockets-addresses
    Dir(PathBuf),
    /// The same as UnixDir, except that on platforms with abstract sockets, the server may attempt
    /// to create an abstract socket whose name starts with this directory instead of a path-based
    /// socket.
    ///
    /// This address is mostly relevant to server (typically bus broker) implementations.
    TmpDir(PathBuf),
}
