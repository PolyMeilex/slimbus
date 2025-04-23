//! Connection API.
use log::trace;
use std::os::fd::{AsFd, AsRawFd, RawFd};
use std::sync::OnceLock;

use crate::{message::Message, names::OwnedUniqueName, Address, Error, Result};

pub mod socket;
pub use socket::Socket;

mod socket_reader;
pub use socket_reader::SocketReader;

pub(crate) mod handshake;
use handshake::Authenticated;

#[derive(Debug)]
pub struct Connection {
    cap_unix_fd: bool,
    unique_name: OnceLock<OwnedUniqueName>,

    socket_write: Box<dyn socket::WriteHalf>,
    raw_fd: RawFd,
}

impl Connection {
    /// Send `msg` to the peer.
    pub fn send(&mut self, msg: &Message) -> Result<()> {
        let data = msg.data();
        if !data.fds().is_empty() && !self.cap_unix_fd {
            return Err(Error::Unsupported);
        }
        let serial = msg.primary_header().serial_num();

        trace!("Sending message: {:?}", msg);
        let write = &mut self.socket_write;
        let mut pos = 0;
        while pos < data.len() {
            let fds = if pos == 0 {
                data.fds().iter().map(|f| f.as_fd()).collect()
            } else {
                vec![]
            };
            pos += write.sendmsg(&data[pos..], &fds)?;
        }
        trace!("Sent message with serial: {}", serial);

        Ok(())
    }

    /// The unique name of the connection, if set/applicable.
    ///
    /// The unique name is assigned by the message bus or set manually using
    /// [`Connection::set_unique_name`].
    pub fn unique_name(&self) -> Option<&OwnedUniqueName> {
        self.unique_name.get()
    }

    pub(crate) fn new(auth: Authenticated, raw_fd: RawFd) -> Result<Self> {
        let cap_unix_fd = auth.cap_unix_fd;

        let connection = Self {
            socket_write: auth.socket_write,
            cap_unix_fd,
            unique_name: OnceLock::new(),
            raw_fd,
        };

        Ok(connection)
    }

    /// Create a `Connection` to the session/user message bus.
    pub fn session() -> Result<(Self, SocketReader)> {
        build(Address::session()?)
    }

    /// Create a `Connection` to the system-wide message bus.
    pub fn system() -> Result<(Self, SocketReader)> {
        build(Address::system()?)
    }
}

impl AsRawFd for Connection {
    fn as_raw_fd(&self) -> RawFd {
        self.raw_fd
    }
}

/// Build the connection, consuming the builder.
///
/// # Errors
///
/// Until server-side bus connection is supported, attempting to build such a connection will
/// result in [`Error::Unsupported`] error.
pub fn build(address: Address) -> Result<(Connection, SocketReader)> {
    let server_guid = address.guid().map(|g| g.to_owned().into());
    let stream = address.connect()?;
    let raw_fd = stream.as_raw_fd();

    let mut auth = Authenticated::client(stream.into(), server_guid)?;

    // SAFETY: `Authenticated` is always built with these fields set to `Some`.
    let socket_read = auth.socket_read.take().unwrap();
    let already_received_bytes = auth.already_received_bytes.take().unwrap();

    let conn = Connection::new(auth, raw_fd)?;

    let reader = SocketReader::new(socket_read, already_received_bytes);

    Ok((conn, reader))
}
