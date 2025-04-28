use rustix::fs::{OFlags, Timespec};
use std::os::fd::{BorrowedFd, RawFd};

mod error;
pub use error::*;

pub mod address;
pub use address::Address;

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

pub mod names;
pub use names::*;

pub use zvariant;

pub fn set_blocking(fd: RawFd, blocking: bool) -> rustix::io::Result<()> {
    let fd = unsafe { BorrowedFd::borrow_raw(fd) };

    // Save the current flags
    let mut flags = rustix::fs::fcntl_getfl(fd)?;

    if blocking {
        flags &= !OFlags::NONBLOCK;
    } else {
        flags |= OFlags::NONBLOCK;
    }

    rustix::fs::fcntl_setfl(fd, flags)?;

    Ok(())
}

pub fn poll(fd: RawFd, timeout: Option<&Timespec>) -> rustix::io::Result<()> {
    let fd = unsafe { BorrowedFd::borrow_raw(fd) };

    let pool_fd = rustix::event::PollFd::new(&fd, rustix::event::PollFlags::IN);
    let mut pool_fds = [pool_fd];

    rustix::event::poll(&mut pool_fds, timeout)?;
    Ok(())
}
