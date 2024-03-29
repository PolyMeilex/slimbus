mod dbus_error;
use std::os::fd::RawFd;

pub use dbus_error::*;

mod error;
pub use error::*;

pub mod address;
pub use address::Address;

mod guid;
pub use guid::*;

pub mod message;
pub use message::Message;

use message::EndianSig;

pub mod connection;
/// Alias for `connection` module, for convenience.
pub use connection as conn;
pub use connection::{handshake::AuthMechanism, Connection, SocketReader};

mod utils;
pub use utils::*;

#[macro_use]
pub mod fdo;

#[deprecated(since = "4.0.0", note = "Use `connection::Socket` instead")]
#[doc(hidden)]
pub use connection::Socket;

// Required for the macros to function within this crate.
extern crate self as zbus;

pub use zbus_names as names;
pub use zvariant;

use nix::libc;

pub fn set_blocking(fd: RawFd, blocking: bool) {
    // Save the current flags
    let mut flags = unsafe { libc::fcntl(fd, libc::F_GETFL, 0) };
    if flags == -1 {
        return;
    }

    if blocking {
        flags &= !libc::O_NONBLOCK;
    } else {
        flags |= libc::O_NONBLOCK;
    }

    let _ = unsafe { libc::fcntl(fd, libc::F_SETFL, flags) != -1 };
}

pub fn poll(fd: RawFd, timeout: i32) {
    let fd = libc::pollfd {
        fd,
        events: libc::POLLIN,
        revents: 0,
    };

    let mut fds = [fd];

    unsafe {
        libc::poll(fds.as_mut_ptr(), fds.len() as u64, timeout);
    }
}
