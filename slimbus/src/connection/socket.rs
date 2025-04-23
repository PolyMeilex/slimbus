use std::{
    io::{self, IoSlice, IoSliceMut},
    os::{
        fd::OwnedFd,
        unix::{
            io::{AsRawFd, BorrowedFd, FromRawFd, RawFd},
            net::UnixStream,
        },
    },
    sync::Arc,
};

use nix::{
    cmsg_space,
    sys::socket::{recvmsg, sendmsg, ControlMessage, ControlMessageOwned, MsgFlags, UnixAddr},
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
    let mut iov = [IoSliceMut::new(buffer)];
    let mut cmsgspace = cmsg_space!([RawFd; FDS_MAX]);

    let msg = recvmsg::<UnixAddr>(fd, &mut iov, Some(&mut cmsgspace), MsgFlags::empty())?;
    if msg.bytes == 0 {
        return Err(io::Error::new(
            io::ErrorKind::BrokenPipe,
            "failed to read from socket",
        ));
    }
    let mut fds = vec![];
    for cmsg in msg.cmsgs()? {
        #[cfg(any(target_os = "freebsd", target_os = "dragonfly"))]
        if let ControlMessageOwned::ScmCreds(_) = cmsg {
            continue;
        }
        if let ControlMessageOwned::ScmRights(fd) = cmsg {
            fds.extend(fd.iter().map(|&f| unsafe { OwnedFd::from_raw_fd(f) }));
        } else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "unexpected CMSG kind",
            ));
        }
    }
    Ok((msg.bytes, fds))
}

fn fd_sendmsg(fd: RawFd, buffer: &[u8], fds: &[BorrowedFd<'_>]) -> io::Result<usize> {
    // FIXME: Remove this conversion once nix supports BorrowedFd here.
    //
    // Tracking issue: https://github.com/nix-rust/nix/issues/1750
    let fds: Vec<_> = fds.iter().map(|f| f.as_raw_fd()).collect();
    let cmsg = if !fds.is_empty() {
        vec![ControlMessage::ScmRights(&fds)]
    } else {
        vec![]
    };
    let iov = [IoSlice::new(buffer)];
    match sendmsg::<UnixAddr>(fd, &iov, &cmsg, MsgFlags::empty(), None) {
        // can it really happen?
        Ok(0) => Err(io::Error::new(
            io::ErrorKind::WriteZero,
            "failed to write to buffer",
        )),
        Ok(n) => Ok(n),
        Err(e) => Err(e.into()),
    }
}

fn get_unix_peer_creds(fd: &impl AsRawFd) -> io::Result<crate::fdo::ConnectionCredentials> {
    let fd = fd.as_raw_fd();
    get_unix_peer_creds_blocking(fd)
}

fn get_unix_peer_creds_blocking(fd: RawFd) -> io::Result<crate::fdo::ConnectionCredentials> {
    #[cfg(any(target_os = "android", target_os = "linux"))]
    {
        use nix::sys::socket::{getsockopt, sockopt::PeerCredentials};

        // TODO: get this BorrowedFd directly from get_unix_peer_creds(), but this requires a
        // 'static lifetime due to the Task.
        let fd = unsafe { BorrowedFd::borrow_raw(fd) };

        getsockopt(&fd, PeerCredentials)
            .map(|creds| {
                crate::fdo::ConnectionCredentials::default()
                    .set_process_id(creds.pid() as _)
                    .set_unix_user_id(creds.uid())
            })
            .map_err(|e| e.into())
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
        let fd = fd.as_raw_fd();
        let uid = nix::unistd::getpeereid(fd).map(|(uid, _)| uid.into())?;
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
    let iov = [std::io::IoSlice::new(b"\0")];
    sendmsg::<()>(
        fd,
        &iov,
        &[ControlMessage::ScmCreds],
        MsgFlags::empty(),
        None,
    )
    .map_err(|e| e.into())
}
