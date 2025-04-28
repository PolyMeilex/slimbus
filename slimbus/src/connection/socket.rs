use std::{
    io::{self, IoSlice, IoSliceMut},
    mem::MaybeUninit,
    os::{
        fd::OwnedFd,
        unix::{
            io::{AsRawFd, BorrowedFd, RawFd},
            net::UnixStream,
        },
    },
    sync::Arc,
};

use rustix::net::{
    RecvAncillaryBuffer, RecvAncillaryMessage, SendAncillaryBuffer, SendAncillaryMessage, SendFlags,
};

type RecvmsgResult = io::Result<(usize, Vec<OwnedFd>)>;

use crate::utils::FDS_MAX;

#[derive(Debug)]
pub struct UnixStreamRead(Arc<UnixStream>);

impl UnixStreamRead {
    pub fn new(v: Arc<UnixStream>) -> Self {
        Self(v)
    }

    pub fn recvmsg(&mut self, buf: &mut [u8]) -> RecvmsgResult {
        loop {
            match fd_recvmsg(self.0.as_raw_fd(), buf) {
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                v => break v,
            }
        }
    }

    pub fn peer_credentials(&mut self) -> io::Result<crate::fdo::ConnectionCredentials> {
        get_unix_peer_creds(&self.0)
    }
}

#[derive(Debug)]
pub struct UnixStreamWrite(Arc<UnixStream>);

impl UnixStreamWrite {
    pub fn new(v: Arc<UnixStream>) -> Self {
        Self(v)
    }

    pub fn sendmsg(&mut self, buffer: &[u8], fds: &[BorrowedFd<'_>]) -> io::Result<usize> {
        loop {
            match fd_sendmsg(self.0.as_raw_fd(), buffer, fds) {
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                v => break v,
            }
        }
    }

    pub fn close(&mut self) -> io::Result<()> {
        let stream = self.0.clone();
        stream.shutdown(std::net::Shutdown::Both)
    }

    #[cfg(any(target_os = "freebsd", target_os = "dragonfly"))]
    pub fn send_zero_byte(&mut self) -> io::Result<Option<usize>> {
        send_zero_byte(&self.0).map(Some)
    }

    /// Supports passing file descriptors.
    pub fn can_pass_unix_fd(&self) -> bool {
        true
    }

    pub fn peer_credentials(&mut self) -> io::Result<crate::fdo::ConnectionCredentials> {
        get_unix_peer_creds(&self.0)
    }
}

fn fd_recvmsg(fd: RawFd, buffer: &mut [u8]) -> io::Result<(usize, Vec<OwnedFd>)> {
    let fd = unsafe { BorrowedFd::borrow_raw(fd) };

    let mut iov = [IoSliceMut::new(buffer)];

    let mut space = [MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(FDS_MAX))];
    let mut cmsg_buffer = RecvAncillaryBuffer::new(&mut space);

    let msg = rustix::net::recvmsg(
        fd,
        &mut iov,
        &mut cmsg_buffer,
        rustix::net::RecvFlags::empty(),
    )?;

    if msg.bytes == 0 {
        return Err(io::Error::new(
            io::ErrorKind::BrokenPipe,
            "failed to read from socket",
        ));
    }

    let fds: Vec<_> = cmsg_buffer
        .drain()
        .filter_map(|cmsg| match cmsg {
            RecvAncillaryMessage::ScmRights(fds) => Some(fds),
            _ => None,
        })
        .flatten()
        .collect();

    Ok((msg.bytes, fds))
}

fn fd_sendmsg(fd: RawFd, buffer: &[u8], fds: &[BorrowedFd<'_>]) -> io::Result<usize> {
    let fd = unsafe { BorrowedFd::borrow_raw(fd) };
    let iov = [IoSlice::new(buffer)];

    let mut space = if !fds.is_empty() {
        vec![MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(fds.len()))]
    } else {
        vec![]
    };

    let mut buffer = SendAncillaryBuffer::new(&mut space);
    if !fds.is_empty() {
        buffer.push(SendAncillaryMessage::ScmRights(fds));
    }

    match rustix::net::sendmsg(fd, &iov, &mut buffer, SendFlags::empty())? {
        // can it really happen?
        0 => Err(io::Error::new(
            io::ErrorKind::WriteZero,
            "failed to write to buffer",
        )),
        n => Ok(n),
    }
}

fn get_unix_peer_creds(fd: &impl AsRawFd) -> io::Result<crate::fdo::ConnectionCredentials> {
    let fd = fd.as_raw_fd();
    get_unix_peer_creds_blocking(fd)
}

fn get_unix_peer_creds_blocking(fd: RawFd) -> io::Result<crate::fdo::ConnectionCredentials> {
    // TODO: get this BorrowedFd directly from get_unix_peer_creds(), but this requires a
    // 'static lifetime due to the Task.
    let fd = unsafe { BorrowedFd::borrow_raw(fd) };

    #[cfg(any(target_os = "android", target_os = "linux"))]
    {
        let creds = rustix::net::sockopt::socket_peercred(fd)?;
        Ok(crate::fdo::ConnectionCredentials::default()
            .set_process_id(creds.pid.as_raw_nonzero().get() as u32)
            .set_unix_user_id(creds.uid.as_raw() as u32))
    }

    #[cfg(any(
        target_os = "macos",
        target_os = "ios",
        target_os = "freebsd",
        target_os = "dragonfly",
        target_os = "openbsd",
        target_os = "netbsd"
    ))]
    {
        let uid = nix::unistd::getpeereid(fd)
            .map(|(uid, _)| uid.into())
            .map_err(|e| io::Error::from_raw_os_error(e as i32))?;
        // FIXME: Handle pid fetching too.
        Ok(crate::fdo::ConnectionCredentials::default().set_unix_user_id(uid))
    }
}

// Send 0 byte as a separate SCM_CREDS message.
#[cfg(any(target_os = "freebsd", target_os = "dragonfly"))]
fn send_zero_byte(fd: &impl AsRawFd) -> io::Result<usize> {
    let fd = fd.as_raw_fd();
    send_zero_byte_blocking(fd)
}

#[cfg(any(target_os = "freebsd", target_os = "dragonfly"))]
fn send_zero_byte_blocking(fd: RawFd) -> io::Result<usize> {
    use nix::sys::socket;

    let iov = [std::io::IoSlice::new(b"\0")];
    socket::sendmsg::<()>(
        fd,
        &iov,
        &[socket::ControlMessage::ScmCreds],
        socket::MsgFlags::empty(),
        None,
    )
    .map_err(|e| io::Error::from_raw_os_error(e as i32))?;
}
