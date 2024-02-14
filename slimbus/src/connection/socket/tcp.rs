use std::io::{self, Read, Write};
use std::os::fd::BorrowedFd;
use std::{net::TcpStream, sync::Arc};

use super::{ReadHalf, RecvmsgResult, WriteHalf};

impl ReadHalf for Arc<TcpStream> {
    fn recvmsg(&mut self, buf: &mut [u8]) -> RecvmsgResult {
        match self.as_ref().read(buf) {
            Err(e) => Err(e),
            Ok(len) => {
                let ret = (len, vec![]);
                Ok(ret)
            }
        }
    }

    fn peer_credentials(&mut self) -> io::Result<crate::fdo::ConnectionCredentials> {
        let creds = crate::fdo::ConnectionCredentials::default();
        Ok(creds)
    }
}

impl WriteHalf for Arc<TcpStream> {
    fn sendmsg(&mut self, buf: &[u8], fds: &[BorrowedFd<'_>]) -> io::Result<usize> {
        if !fds.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "fds cannot be sent with a tcp stream",
            ));
        }

        self.as_ref().write(buf)
    }

    fn close(&mut self) -> io::Result<()> {
        let stream = self.clone();
        stream.shutdown(std::net::Shutdown::Both)
    }

    fn peer_credentials(&mut self) -> io::Result<crate::fdo::ConnectionCredentials> {
        ReadHalf::peer_credentials(self)
    }
}
