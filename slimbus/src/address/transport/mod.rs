//! D-Bus transport Information module.
//!
//! This module provides the trasport information for D-Bus addresses.

use crate::{Error, Result};
use std::collections::HashMap;
use std::net::TcpStream;
use std::os::unix::net::{SocketAddr, UnixStream};

use std::{
    fmt::{Display, Formatter},
    str::from_utf8_unchecked,
};

mod unix;
pub use unix::{Unix, UnixSocket};
mod tcp;
#[cfg(target_os = "linux")]
use std::os::linux::net::SocketAddrExt;
pub use tcp::{Tcp, TcpTransportFamily};

/// The transport properties of a D-Bus address.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Transport {
    /// A Unix Domain Socket address.
    Unix(Unix),
    /// TCP address details
    Tcp(Tcp),
}

impl Transport {
    pub(super) fn connect(self) -> Result<Stream> {
        match self {
            Transport::Unix(unix) => {
                let addr = match unix.take_path() {
                    UnixSocket::File(path) => SocketAddr::from_pathname(path)?,
                    #[cfg(target_os = "linux")]
                    UnixSocket::Abstract(name) => {
                        SocketAddr::from_abstract_name(name.as_encoded_bytes())?
                    }
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

                Ok(Stream::Unix(stream))
            }

            Transport::Tcp(mut addr) => match addr.take_nonce_file() {
                Some(nonce_file) => {
                    let mut stream = addr.connect()?;

                    let nonce_file = {
                        use std::os::unix::ffi::OsStrExt;
                        std::ffi::OsStr::from_bytes(&nonce_file)
                    };

                    let nonce = std::fs::read(nonce_file)?;
                    std::io::Write::write_all(&mut stream, &nonce)?;

                    Ok(Stream::Tcp(stream))
                }
                None => addr.connect().map(Stream::Tcp),
            },
        }
    }

    // Helper for `FromStr` impl of `Address`.
    pub(super) fn from_options(transport: &str, options: HashMap<&str, &str>) -> Result<Self> {
        match transport {
            "unix" => Unix::from_options(options).map(Self::Unix),
            "tcp" => Tcp::from_options(options, false).map(Self::Tcp),
            "nonce-tcp" => Tcp::from_options(options, true).map(Self::Tcp),
            _ => Err(Error::Address(format!(
                "unsupported transport '{transport}'"
            ))),
        }
    }
}

#[derive(Debug)]
pub(crate) enum Stream {
    Unix(UnixStream),
    Tcp(TcpStream),
}

fn decode_hex(c: char) -> Result<u8> {
    match c {
        '0'..='9' => Ok(c as u8 - b'0'),
        'a'..='f' => Ok(c as u8 - b'a' + 10),
        'A'..='F' => Ok(c as u8 - b'A' + 10),

        _ => Err(Error::Address(
            "invalid hexadecimal character in percent-encoded sequence".to_owned(),
        )),
    }
}

pub(crate) fn decode_percents(value: &str) -> Result<Vec<u8>> {
    let mut iter = value.chars();
    let mut decoded = Vec::new();

    while let Some(c) = iter.next() {
        if matches!(c, '-' | '0'..='9' | 'A'..='Z' | 'a'..='z' | '_' | '/' | '.' | '\\' | '*') {
            decoded.push(c as u8)
        } else if c == '%' {
            decoded.push(
                decode_hex(iter.next().ok_or_else(|| {
                    Error::Address("incomplete percent-encoded sequence".to_owned())
                })?)?
                    << 4
                    | decode_hex(iter.next().ok_or_else(|| {
                        Error::Address("incomplete percent-encoded sequence".to_owned())
                    })?)?,
            );
        } else {
            return Err(Error::Address("Invalid character in address".to_owned()));
        }
    }

    Ok(decoded)
}

pub(super) fn encode_percents(f: &mut Formatter<'_>, mut value: &[u8]) -> std::fmt::Result {
    const LOOKUP: &str = "\
%00%01%02%03%04%05%06%07%08%09%0a%0b%0c%0d%0e%0f\
%10%11%12%13%14%15%16%17%18%19%1a%1b%1c%1d%1e%1f\
%20%21%22%23%24%25%26%27%28%29%2a%2b%2c%2d%2e%2f\
%30%31%32%33%34%35%36%37%38%39%3a%3b%3c%3d%3e%3f\
%40%41%42%43%44%45%46%47%48%49%4a%4b%4c%4d%4e%4f\
%50%51%52%53%54%55%56%57%58%59%5a%5b%5c%5d%5e%5f\
%60%61%62%63%64%65%66%67%68%69%6a%6b%6c%6d%6e%6f\
%70%71%72%73%74%75%76%77%78%79%7a%7b%7c%7d%7e%7f\
%80%81%82%83%84%85%86%87%88%89%8a%8b%8c%8d%8e%8f\
%90%91%92%93%94%95%96%97%98%99%9a%9b%9c%9d%9e%9f\
%a0%a1%a2%a3%a4%a5%a6%a7%a8%a9%aa%ab%ac%ad%ae%af\
%b0%b1%b2%b3%b4%b5%b6%b7%b8%b9%ba%bb%bc%bd%be%bf\
%c0%c1%c2%c3%c4%c5%c6%c7%c8%c9%ca%cb%cc%cd%ce%cf\
%d0%d1%d2%d3%d4%d5%d6%d7%d8%d9%da%db%dc%dd%de%df\
%e0%e1%e2%e3%e4%e5%e6%e7%e8%e9%ea%eb%ec%ed%ee%ef\
%f0%f1%f2%f3%f4%f5%f6%f7%f8%f9%fa%fb%fc%fd%fe%ff";

    loop {
        let pos = value.iter().position(
            |c| !matches!(c, b'-' | b'0'..=b'9' | b'A'..=b'Z' | b'a'..=b'z' | b'_' | b'/' | b'.' | b'\\' | b'*'),
        );

        if let Some(pos) = pos {
            // SAFETY: The above `position()` call made sure that only ASCII chars are in the string
            // up to `pos`
            f.write_str(unsafe { from_utf8_unchecked(&value[..pos]) })?;

            let c = value[pos];
            value = &value[pos + 1..];

            let pos = c as usize * 3;
            f.write_str(&LOOKUP[pos..pos + 3])?;
        } else {
            // SAFETY: The above `position()` call made sure that only ASCII chars are in the rest
            // of the string
            f.write_str(unsafe { from_utf8_unchecked(value) })?;
            return Ok(());
        }
    }
}

impl Display for Transport {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Tcp(tcp) => write!(f, "{}", tcp)?,
            Self::Unix(unix) => write!(f, "{}", unix)?,
        }

        Ok(())
    }
}
