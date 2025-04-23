//! D-Bus transport Information module.
//!
//! This module provides the transport information for D-Bus addresses.

use crate::{Error, Result};
use std::collections::HashMap;
use std::os::unix::net::{SocketAddr, UnixStream};

mod unix;
pub use unix::{Unix, UnixSocket};

#[cfg(target_os = "linux")]
use std::os::linux::net::SocketAddrExt;

/// The transport properties of a D-Bus address.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Transport(
    // A Unix Domain Socket address.
    Unix,
);

impl Transport {
    pub(super) fn connect(self) -> Result<UnixStream> {
        let unix = self.0;

        let addr = match unix.take_path() {
            UnixSocket::File(path) => SocketAddr::from_pathname(path)?,
            #[cfg(target_os = "linux")]
            UnixSocket::Abstract(name) => SocketAddr::from_abstract_name(name.as_encoded_bytes())?,
            UnixSocket::Dir(_) | UnixSocket::TmpDir(_) => {
                // you can't connect to a unix:dir
                return Err(Error::Unsupported);
            }
        };
        let stream = {
            let stream = UnixStream::connect_addr(&addr)?;
            stream.set_nonblocking(false)?;
            stream
        };

        Ok(stream)
    }

    // Helper for `FromStr` impl of `Address`.
    pub(super) fn from_options(transport: &str, options: HashMap<&str, &str>) -> Result<Self> {
        match transport {
            "unix" => Unix::from_options(options).map(Self),
            _ => Err(Error::Address(format!(
                "unsupported transport '{transport}'"
            ))),
        }
    }
}
