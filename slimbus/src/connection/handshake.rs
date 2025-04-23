use log::trace;
use std::{
    fmt::{self, Debug},
    os::unix::net::UnixStream,
    str::FromStr,
    sync::Arc,
};

use crate::{guid::Guid, Error, OwnedGuid, Result};

use super::socket::{UnixStreamRead, UnixStreamWrite};

/// Authentication mechanisms
///
/// See <https://dbus.freedesktop.org/doc/dbus-specification.html#auth-mechanisms>
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuthMechanism {
    /// This is the recommended authentication mechanism on platforms where credentials can be
    /// transferred out-of-band, in particular Unix platforms that can perform credentials-passing
    /// over the `unix:` transport.
    External,
}

/// The result of a finalized handshake
///
/// The result of a finalized [`ClientHandshake`] or [`ServerHandshake`]. It can be passed to
/// [`Connection::new_authenticated`] to initialize a connection.
///
/// [`ClientHandshake`]: struct.ClientHandshake.html
/// [`ServerHandshake`]: struct.ServerHandshake.html
/// [`Connection::new_authenticated`]: ../struct.Connection.html#method.new_authenticated
#[derive(Debug)]
pub struct Authenticated {
    pub(crate) socket_write: UnixStreamWrite,
    /// Whether file descriptor passing has been accepted by both sides
    pub(crate) cap_unix_fd: bool,

    pub(crate) socket_read: Option<UnixStreamRead>,
    pub(crate) already_received_bytes: Option<Vec<u8>>,
}

impl Authenticated {
    /// Create a client-side `Authenticated` for the given `socket`.
    pub fn client(socket: UnixStream, server_guid: Option<OwnedGuid>) -> Result<Self> {
        ClientHandshake::new(socket, server_guid).perform()
    }
}

/*
 * Client-side handshake logic
 */

// The plain-text SASL profile authentication protocol described here:
// <https://dbus.freedesktop.org/doc/dbus-specification.html#auth-protocol>
//
// These are all the known commands, which can be parsed from or serialized to text.
#[derive(Debug)]
#[allow(clippy::upper_case_acronyms)]
enum Command {
    Auth(Option<AuthMechanism>, Option<Vec<u8>>),
    Cancel,
    Begin,
    Error(String),
    NegotiateUnixFD,
    Rejected(Vec<AuthMechanism>),
    Ok(OwnedGuid),
    AgreeUnixFD,
}

/// A representation of an in-progress handshake, client-side
///
/// This struct is an async-compatible representation of the initial handshake that must be
/// performed before a D-Bus connection can be used. To use it, you should call the
/// [`advance_handshake`] method whenever the underlying socket becomes ready (tracking the
/// readiness itself is not managed by this abstraction) until it returns `Ok(())`, at which point
/// you can invoke the [`try_finish`] method to get an [`Authenticated`], which can be given to
/// [`Connection::new_authenticated`].
///
/// [`advance_handshake`]: struct.ClientHandshake.html#method.advance_handshake
/// [`try_finish`]: struct.ClientHandshake.html#method.try_finish
/// [`Authenticated`]: struct.AUthenticated.html
/// [`Connection::new_authenticated`]: ../struct.Connection.html#method.new_authenticated
#[derive(Debug)]
pub struct ClientHandshake {
    common: HandshakeCommon,
    server_guid: Option<OwnedGuid>,
}

fn sasl_auth_id() -> String {
    unsafe { nix::libc::geteuid() }.to_string()
}

fn bytes_to_hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    // Each byte becomes two hex digits.
    let mut s = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        // Write two-character, lowercase hex (e.g. "0f", "a3").
        write!(&mut s, "{:02x}", b).expect("Writing to String should never fail");
    }
    s
}

impl ClientHandshake {
    /// Start a handshake on this client socket
    pub fn new(socket: UnixStream, server_guid: Option<OwnedGuid>) -> ClientHandshake {
        ClientHandshake {
            common: HandshakeCommon::new(socket),
            server_guid,
        }
    }

    fn handle_init(&mut self) -> Result<()> {
        trace!("Initializing");

        // The dbus daemon on some platforms requires sending the zero byte as a
        // separate message with SCM_CREDS.
        #[cfg(any(target_os = "freebsd", target_os = "dragonfly"))]
        let written = self
            .common
            .socket
            .write_mut()
            .send_zero_byte()
            .map_err(|e| {
                Error::Handshake(format!("Could not send zero byte with credentials: {}", e))
            })
            .and_then(|n| match n {
                None => Err(Error::Handshake(
                    "Could not send zero byte with credentials".to_string(),
                )),
                Some(n) => Ok(n),
            })?;

        // leading 0 is sent separately already for `freebsd` and `dragonfly` above.
        #[cfg(not(any(target_os = "freebsd", target_os = "dragonfly")))]
        let written = self.common.socket_write.sendmsg(b"\0", &[])?;

        if written != 1 {
            return Err(Error::Handshake(
                "Could not send zero byte with credentials".to_string(),
            ));
        }

        Ok(())
    }

    fn wait_for_ok(&mut self) -> Result<()> {
        trace!("Waiting for DATA or OK from server");

        match self.common.read_command()? {
            Command::Ok(guid) => {
                trace!("Received OK from server");
                match self.server_guid.clone() {
                    Some(server_guid) if server_guid != guid => {
                        return Err(Error::Handshake(format!(
                            "Server GUID mismatch: expected {server_guid}, got {guid}",
                        )));
                    }
                    Some(_) => (),
                    None => self.server_guid = Some(guid),
                }

                Ok(())
            }
            Command::Rejected(_) => {
                trace!("Received REJECT from server. Will try next auth mechanism..");
                Err(Error::Handshake(
                    "Exhausted available AUTH mechanisms".into(),
                ))
            }
            reply => Err(Error::Handshake(format!(
                "Unexpected server AUTH OK reply: {reply}"
            ))),
        }
    }

    fn wait_for_agree_unix_fd(&mut self) -> Result<()> {
        trace!("Waiting for Unix FD passing agreement from server");

        match self.common.read_command()? {
            Command::AgreeUnixFD => {
                trace!("Unix FD passing agreed by server");
                self.common.cap_unix_fd = true
            }
            Command::Error(_) => {
                trace!("Unix FD passing rejected by server");
                self.common.cap_unix_fd = false
            }
            replay => {
                return Err(Error::Handshake(format!(
                    "Unexpected server UNIX_FD reply: {replay}"
                )));
            }
        }

        Ok(())
    }

    /// Perform the handshake.
    ///
    /// On a successful handshake, you get an `Authenticated`. If you need to send a Bus Hello,
    /// this remains to be done.
    fn perform(mut self) -> Result<Authenticated> {
        self.handle_init()?;
        self.common.write_command(Command::Auth(
            Some(AuthMechanism::External),
            Some(sasl_auth_id().into_bytes()),
        ))?;

        self.wait_for_ok()?;

        self.common.write_command(Command::NegotiateUnixFD)?;

        self.wait_for_agree_unix_fd()?;

        self.common.write_command(Command::Begin)?;

        trace!("Handshake done");

        Ok(Authenticated {
            socket_write: self.common.socket_write,
            socket_read: Some(self.common.socket_read),
            cap_unix_fd: self.common.cap_unix_fd,
            already_received_bytes: Some(self.common.recv_buffer),
        })
    }
}

/*
 * Server-side handshake logic
 */

// A representation of an in-progress handshake, server-side

impl fmt::Display for AuthMechanism {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mech = match self {
            AuthMechanism::External => "EXTERNAL",
        };
        write!(f, "{mech}")
    }
}

impl FromStr for AuthMechanism {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        match s {
            "EXTERNAL" => Ok(AuthMechanism::External),
            _ => Err(Error::Handshake(format!("Unknown mechanism: {s}"))),
        }
    }
}

impl From<Command> for Vec<u8> {
    fn from(c: Command) -> Self {
        c.to_string().into()
    }
}

impl fmt::Display for Command {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Command::Auth(mech, resp) => match (mech, resp) {
                (Some(mech), Some(resp)) => write!(f, "AUTH {mech} {}", bytes_to_hex(resp)),
                (Some(mech), None) => write!(f, "AUTH {mech}"),
                _ => write!(f, "AUTH"),
            },
            Command::Cancel => write!(f, "CANCEL"),
            Command::Begin => write!(f, "BEGIN"),
            Command::Error(expl) => write!(f, "ERROR {expl}"),
            Command::NegotiateUnixFD => write!(f, "NEGOTIATE_UNIX_FD"),
            Command::Rejected(mechs) => {
                write!(
                    f,
                    "REJECTED {}",
                    mechs
                        .iter()
                        .map(|m| m.to_string())
                        .collect::<Vec<_>>()
                        .join(" ")
                )
            }
            Command::Ok(guid) => write!(f, "OK {guid}"),
            Command::AgreeUnixFD => write!(f, "AGREE_UNIX_FD"),
        }?;
        write!(f, "\r\n")
    }
}

impl FromStr for Command {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        let mut words = s.split_ascii_whitespace();
        let cmd = match words.next() {
            Some("CANCEL") => Command::Cancel,
            Some("BEGIN") => Command::Begin,
            Some("ERROR") => Command::Error(s.into()),
            Some("NEGOTIATE_UNIX_FD") => Command::NegotiateUnixFD,
            Some("REJECTED") => {
                let mechs = words.map(|m| m.parse()).collect::<Result<_>>()?;
                Command::Rejected(mechs)
            }
            Some("OK") => {
                let guid = words
                    .next()
                    .ok_or_else(|| Error::Handshake("Missing OK server GUID!".into()))?;
                Command::Ok(Guid::from_str(guid)?.into())
            }
            Some("AGREE_UNIX_FD") => Command::AgreeUnixFD,
            _ => return Err(Error::Handshake(format!("Unknown command: {s}"))),
        };
        Ok(cmd)
    }
}

// Common code for the client and server side of the handshake.
#[derive(Debug)]
pub struct HandshakeCommon {
    socket_read: UnixStreamRead,
    socket_write: UnixStreamWrite,
    recv_buffer: Vec<u8>,
    cap_unix_fd: bool,
}

impl HandshakeCommon {
    /// Start a handshake on this client socket
    pub fn new(socket: UnixStream) -> Self {
        let socket = Arc::new(socket);
        Self {
            socket_read: UnixStreamRead::new(socket.clone()),
            socket_write: UnixStreamWrite::new(socket),
            recv_buffer: Vec::new(),
            cap_unix_fd: false,
        }
    }

    fn write_command(&mut self, command: Command) -> Result<()> {
        let mut send_buffer = Vec::<u8>::from(command);
        while !send_buffer.is_empty() {
            let written = self.socket_write.sendmsg(&send_buffer, &[])?;
            send_buffer.drain(..written);
        }
        Ok(())
    }

    fn read_command(&mut self) -> Result<Command> {
        let mut cmd_end = 0;
        loop {
            if let Some(i) = self.recv_buffer[cmd_end..].iter().position(|b| *b == b'\n') {
                if cmd_end + i == 0 || self.recv_buffer.get(cmd_end + i - 1) != Some(&b'\r') {
                    return Err(Error::Handshake("Invalid line ending in handshake".into()));
                }
                cmd_end += i + 1;

                break;
            } else {
                cmd_end = self.recv_buffer.len();
            }

            let mut buf = [0; 64];
            let res = self.socket_read.recvmsg(&mut buf)?;
            let read = {
                let (read, fds) = res;
                if !fds.is_empty() {
                    return Err(Error::Handshake("Unexpected FDs during handshake".into()));
                }
                read
            };
            if read == 0 {
                return Err(Error::Handshake("Unexpected EOF during handshake".into()));
            }
            self.recv_buffer.extend(&buf[..read]);
        }

        let line_bytes = self.recv_buffer.drain(..cmd_end);
        let line = std::str::from_utf8(line_bytes.as_slice())
            .map_err(|e| Error::Handshake(e.to_string()))?;

        trace!("Reading {line}");
        line.parse()
    }
}
