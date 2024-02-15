mod split;
pub use split::{BoxedSplit, Split};

mod tcp;
mod unix;

use std::io;
use std::sync::Arc;

use crate::fdo::ConnectionCredentials;
use std::os::fd::{BorrowedFd, OwnedFd};

type RecvmsgResult = io::Result<(usize, Vec<OwnedFd>)>;

/// Trait representing some transport layer over which the DBus protocol can be used
///
/// In order to allow simultaneous reading and writing, this trait requires you to split the socket
/// into a read half and a write half. The reader and writer halves can be any types that implement
/// [`ReadHalf`] and [`WriteHalf`] respectively.
///
/// The crate provides implementations for `async_io` and `tokio`'s `UnixStream` wrappers if you
/// enable the corresponding crate features (`async_io` is enabled by default).
///
/// You can implement it manually to integrate with other runtimes or other dbus transports.  Feel
/// free to submit pull requests to add support for more runtimes to zbus itself so rust's orphan
/// rules don't force the use of a wrapper struct (and to avoid duplicating the work across many
/// projects).
pub trait Socket {
    type ReadHalf: ReadHalf;
    type WriteHalf: WriteHalf;

    /// Split the socket into a read half and a write half.
    fn split(self) -> Split<Self::ReadHalf, Self::WriteHalf>
    where
        Self: Sized;
}

/// The read half of a socket.
///
/// See [`Socket`] for more details.
pub trait ReadHalf: std::fmt::Debug + Send + Sync + 'static {
    /// Attempt to receive a message from the socket.
    ///
    /// On success, returns the number of bytes read as well as a `Vec` containing
    /// any associated file descriptors.
    fn recvmsg(&mut self, buf: &mut [u8]) -> RecvmsgResult;

    /// Supports passing file descriptors.
    ///
    /// Default implementation returns `false`.
    fn can_pass_unix_fd(&self) -> bool {
        false
    }

    /// Return the peer credentials.
    fn peer_credentials(&mut self) -> io::Result<ConnectionCredentials> {
        Ok(ConnectionCredentials::default())
    }
}

/// The write half of a socket.
///
/// See [`Socket`] for more details.
pub trait WriteHalf: std::fmt::Debug + Send + Sync + 'static {
    /// Attempt to send a message on the socket
    ///
    /// On success, return the number of bytes written. There may be a partial write, in
    /// which case the caller is responsible of sending the remaining data by calling this
    /// method again until everything is written or it returns an error of kind `WouldBlock`.
    ///
    /// If at least one byte has been written, then all the provided file descriptors will
    /// have been sent as well, and should not be provided again in subsequent calls.
    ///
    /// If the underlying transport does not support transmitting file descriptors, this
    /// will return `Err(ErrorKind::InvalidInput)`.
    fn sendmsg(&mut self, buffer: &[u8], fds: &[BorrowedFd<'_>]) -> io::Result<usize>;

    /// The dbus daemon on `freebsd` and `dragonfly` currently requires sending the zero byte
    /// as a separate message with SCM_CREDS, as part of the `EXTERNAL` authentication on unix
    /// sockets. This method is used by the authentication machinery in zbus to send this
    /// zero byte. Socket implementations based on unix sockets should implement this method.
    #[cfg(any(target_os = "freebsd", target_os = "dragonfly"))]
    fn send_zero_byte(&mut self) -> io::Result<Option<usize>> {
        Ok(None)
    }

    /// Close the socket.
    ///
    /// After this call, it is valid for all reading and writing operations to fail.
    fn close(&mut self) -> io::Result<()>;

    /// Supports passing file descriptors.
    ///
    /// Default implementation returns `false`.
    fn can_pass_unix_fd(&self) -> bool {
        false
    }

    /// Return the peer credentials.
    fn peer_credentials(&mut self) -> io::Result<ConnectionCredentials> {
        Ok(ConnectionCredentials::default())
    }
}

impl ReadHalf for Box<dyn ReadHalf> {
    fn can_pass_unix_fd(&self) -> bool {
        (**self).can_pass_unix_fd()
    }

    fn recvmsg(&mut self, buf: &mut [u8]) -> RecvmsgResult {
        (**self).recvmsg(buf)
    }

    fn peer_credentials(&mut self) -> io::Result<ConnectionCredentials> {
        (**self).peer_credentials()
    }
}

impl WriteHalf for Box<dyn WriteHalf> {
    fn sendmsg(&mut self, buffer: &[u8], #[cfg(unix)] fds: &[BorrowedFd<'_>]) -> io::Result<usize> {
        (**self).sendmsg(buffer, fds)
    }

    #[cfg(any(target_os = "freebsd", target_os = "dragonfly"))]
    fn send_zero_byte(&mut self) -> io::Result<Option<usize>> {
        (**self).send_zero_byte()
    }

    fn close(&mut self) -> io::Result<()> {
        (**self).close()
    }

    fn can_pass_unix_fd(&self) -> bool {
        (**self).can_pass_unix_fd()
    }

    fn peer_credentials(&mut self) -> io::Result<ConnectionCredentials> {
        (**self).peer_credentials()
    }
}

impl<T> Socket for T
where
    T: std::fmt::Debug + Send + Sync,
    Arc<T>: ReadHalf + WriteHalf,
{
    type ReadHalf = Arc<T>;
    type WriteHalf = Arc<T>;

    fn split(self) -> Split<Self::ReadHalf, Self::WriteHalf> {
        let arc = Arc::new(self);

        Split {
            read: arc.clone(),
            write: arc,
        }
    }
}
