use log::trace;
use std::{
    collections::VecDeque,
    fmt::{self, Debug},
    io::BufRead,
    path::PathBuf,
    str::FromStr,
};
use zvariant::Str;

fn home_dir() -> Option<PathBuf> {
    match std::env::var("HOME") {
        Ok(home) => Some(home.into()),
        Err(_) => unix::home_dir(),
    }
}

mod unix {
    use std::ffi::{CStr, OsStr};
    use std::os::unix::ffi::OsStrExt;
    use std::path::PathBuf;

    pub(super) fn home_dir() -> Option<PathBuf> {
        let uid = unsafe { nix::libc::geteuid() };
        let passwd = unsafe { nix::libc::getpwuid(uid) };

        // getpwnam(3):
        // The getpwnam() and getpwuid() functions return a pointer to a passwd structure, or NULL
        // if the matching entry is not found or an error occurs. If an error occurs, errno is set
        // to indicate the error. If one wants to check errno after the call, it should be set to
        // zero before the call. The return value may point to a static area, and may be overwritten
        // by subsequent calls to getpwent(3), getpwnam(), or getpwuid().
        if passwd.is_null() {
            return None;
        }

        // SAFETY: `getpwuid()` returns either NULL or a valid pointer to a `passwd` structure.
        let passwd = unsafe { &*passwd };
        if passwd.pw_dir.is_null() {
            return None;
        }

        // SAFETY: `getpwuid()->pw_dir` is a valid pointer to a c-string.
        let home_dir = unsafe { CStr::from_ptr(passwd.pw_dir) };

        Some(PathBuf::from(OsStr::from_bytes(home_dir.to_bytes())))
    }
}

use crate::{guid::Guid, Error, OwnedGuid, Result};

use super::socket::{BoxedSplit, ReadHalf, WriteHalf};

/// Authentication mechanisms
///
/// See <https://dbus.freedesktop.org/doc/dbus-specification.html#auth-mechanisms>
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuthMechanism {
    /// This is the recommended authentication mechanism on platforms where credentials can be
    /// transferred out-of-band, in particular Unix platforms that can perform credentials-passing
    /// over the `unix:` transport.
    External,

    /// This mechanism is designed to establish that a client has the ability to read a private
    /// file owned by the user being authenticated.
    Cookie,

    /// Does not perform any authentication at all, and should not be accepted by message buses.
    /// However, it might sometimes be useful for non-message-bus uses of D-Bus.
    Anonymous,
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
    pub(crate) socket_write: Box<dyn WriteHalf>,
    /// Whether file descriptor passing has been accepted by both sides
    pub(crate) cap_unix_fd: bool,

    pub(crate) socket_read: Option<Box<dyn ReadHalf>>,
    pub(crate) already_received_bytes: Option<Vec<u8>>,
}

impl Authenticated {
    /// Create a client-side `Authenticated` for the given `socket`.
    pub fn client(
        socket: BoxedSplit,
        server_guid: Option<OwnedGuid>,
        mechanisms: Option<VecDeque<AuthMechanism>>,
    ) -> Result<Self> {
        ClientHandshake::new(socket, mechanisms, server_guid).perform()
    }
}

/*
 * Client-side handshake logic
 */

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(clippy::upper_case_acronyms)]
enum ClientHandshakeStep {
    Init,
    MechanismInit,
    WaitingForData,
    WaitingForOK,
    WaitingForAgreeUnixFD,
    Done,
}

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
    Data(Option<Vec<u8>>),
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
    step: ClientHandshakeStep,
}

pub trait Handshake {
    /// Perform the handshake.
    ///
    /// On a successful handshake, you get an `Authenticated`. If you need to send a Bus Hello,
    /// this remains to be done.
    fn perform(self) -> Result<Authenticated>;
}

impl ClientHandshake {
    /// Start a handshake on this client socket
    pub fn new(
        socket: BoxedSplit,
        mechanisms: Option<VecDeque<AuthMechanism>>,
        server_guid: Option<OwnedGuid>,
    ) -> ClientHandshake {
        let mechanisms = mechanisms.unwrap_or_else(|| {
            let mut mechanisms = VecDeque::new();
            mechanisms.push_back(AuthMechanism::External);
            mechanisms.push_back(AuthMechanism::Cookie);
            mechanisms.push_back(AuthMechanism::Anonymous);
            mechanisms
        });

        ClientHandshake {
            common: HandshakeCommon::new(socket, mechanisms),
            step: ClientHandshakeStep::Init,
            server_guid,
        }
    }

    fn mechanism_init(&mut self) -> Result<(ClientHandshakeStep, Command)> {
        use ClientHandshakeStep::*;
        let mech = self.common.mechanism()?;
        match mech {
            AuthMechanism::Anonymous => Ok((
                WaitingForOK,
                Command::Auth(Some(*mech), Some("zbus".into())),
            )),
            AuthMechanism::External => Ok((
                WaitingForOK,
                Command::Auth(Some(*mech), Some(sasl_auth_id()?.into_bytes())),
            )),
            AuthMechanism::Cookie => Ok((
                WaitingForData,
                Command::Auth(Some(*mech), Some(sasl_auth_id()?.into_bytes())),
            )),
        }
    }

    fn mechanism_data(&mut self, data: Vec<u8>) -> Result<(ClientHandshakeStep, Command)> {
        let mech = self.common.mechanism()?;
        match mech {
            AuthMechanism::Cookie => {
                let context = std::str::from_utf8(&data)
                    .map_err(|_| Error::Handshake("Cookie context was not valid UTF-8".into()))?;
                let mut split = context.split_ascii_whitespace();
                let context = split
                    .next()
                    .ok_or_else(|| Error::Handshake("Missing cookie context name".into()))?;
                let context = Str::from(context).try_into()?;
                let id = split
                    .next()
                    .ok_or_else(|| Error::Handshake("Missing cookie ID".into()))?;
                let id = id
                    .parse()
                    .map_err(|e| Error::Handshake(format!("Invalid cookie ID `{id}`: {e}")))?;
                let server_challenge = split
                    .next()
                    .ok_or_else(|| Error::Handshake("Missing cookie challenge".into()))?;

                let cookie = Cookie::lookup(&context, id)?.cookie;
                let client_challenge = random_ascii(16);
                let sec = format!("{server_challenge}:{client_challenge}:{cookie}");
                let sha1 = sha1_smol::Sha1::from(sec).hexdigest();
                let data = format!("{client_challenge} {sha1}");
                Ok((
                    ClientHandshakeStep::WaitingForOK,
                    Command::Data(Some(data.into())),
                ))
            }
            _ => Err(Error::Handshake("Unexpected mechanism DATA".into())),
        }
    }
}

fn random_ascii(len: usize) -> String {
    use rand::{distributions::Alphanumeric, thread_rng, Rng};
    use std::iter;

    let mut rng = thread_rng();
    iter::repeat(())
        .map(|()| rng.sample(Alphanumeric))
        .map(char::from)
        .take(len)
        .collect()
}

fn sasl_auth_id() -> Result<String> {
    let id = unsafe { nix::libc::geteuid() }.to_string();
    Ok(id)
}

#[derive(Debug)]
struct Cookie {
    id: usize,
    cookie: String,
}

impl Cookie {
    fn keyring_path() -> Result<PathBuf> {
        let mut path = home_dir()
            .ok_or_else(|| Error::Handshake("Failed to determine home directory".into()))?;
        path.push(".dbus-keyrings");
        Ok(path)
    }

    fn read_keyring(context: &CookieContext<'_>) -> Result<Vec<Cookie>> {
        let mut path = Cookie::keyring_path()?;
        {
            use std::os::unix::fs::PermissionsExt;

            let perms = std::fs::metadata(&path)?.permissions().mode();
            if perms & 0o066 != 0 {
                return Err(Error::Handshake(
                    "DBus keyring has invalid permissions".into(),
                ));
            }
        }

        path.push(&*context.0);
        trace!("Reading keyring {:?}", path);

        let lines = std::fs::File::open(&path)
            .map(std::io::BufReader::new)
            .map(std::io::BufReader::lines)?;

        let mut cookies = vec![];
        for (n, line) in lines.enumerate() {
            let line = line?;
            let mut split = line.split_whitespace();
            let id = split
                .next()
                .ok_or_else(|| {
                    Error::Handshake(format!(
                        "DBus cookie `{}` missing ID at line {n}",
                        path.display(),
                    ))
                })?
                .parse()
                .map_err(|e| {
                    Error::Handshake(format!(
                        "Failed to parse cookie ID in file `{}` at line {n}: {e}",
                        path.display(),
                    ))
                })?;
            let _ = split.next().ok_or_else(|| {
                Error::Handshake(format!(
                    "DBus cookie `{}` missing creation time at line {n}",
                    path.display(),
                ))
            })?;
            let cookie = split
                .next()
                .ok_or_else(|| {
                    Error::Handshake(format!(
                        "DBus cookie `{}` missing cookie data at line {}",
                        path.to_str().unwrap(),
                        n
                    ))
                })?
                .to_string();
            cookies.push(Cookie { id, cookie })
        }
        trace!("Loaded keyring {:?}", cookies);
        Ok(cookies)
    }

    fn lookup(context: &CookieContext<'_>, id: usize) -> Result<Cookie> {
        let keyring = Self::read_keyring(context)?;
        keyring
            .into_iter()
            .find(|c| c.id == id)
            .ok_or_else(|| Error::Handshake(format!("DBus cookie ID {id} not found")))
    }
}

#[derive(Debug)]
pub struct CookieContext<'c>(Str<'c>);

impl<'c> TryFrom<Str<'c>> for CookieContext<'c> {
    type Error = Error;

    fn try_from(value: Str<'c>) -> Result<Self> {
        if value.is_empty() {
            return Err(Error::Handshake("Empty cookie context".into()));
        } else if !value.is_ascii() || value.contains(['/', '\\', ' ', '\n', '\r', '\t', '.']) {
            return Err(Error::Handshake(
                "Invalid characters in cookie context".into(),
            ));
        }

        Ok(Self(value))
    }
}

impl Default for CookieContext<'_> {
    fn default() -> Self {
        Self(Str::from_static("org_freedesktop_general"))
    }
}

impl Handshake for ClientHandshake {
    fn perform(mut self) -> Result<Authenticated> {
        use ClientHandshakeStep::*;
        loop {
            let (next_step, cmd) = match self.step {
                Init => {
                    trace!("Initializing");
                    #[allow(clippy::let_and_return)]
                    let ret = self.mechanism_init()?;
                    // The dbus daemon on some platforms requires sending the zero byte as a
                    // separate message with SCM_CREDS.
                    #[cfg(any(target_os = "freebsd", target_os = "dragonfly"))]
                    let written = self
                        .common
                        .socket
                        .write_mut()
                        .send_zero_byte()
                        .map_err(|e| {
                            Error::Handshake(format!(
                                "Could not send zero byte with credentials: {}",
                                e
                            ))
                        })
                        .and_then(|n| match n {
                            None => Err(Error::Handshake(
                                "Could not send zero byte with credentials".to_string(),
                            )),
                            Some(n) => Ok(n),
                        })?;

                    // leading 0 is sent separately already for `freebsd` and `dragonfly` above.
                    #[cfg(not(any(target_os = "freebsd", target_os = "dragonfly")))]
                    let written = self.common.socket.write_mut().sendmsg(&[b'\0'], &[])?;

                    if written != 1 {
                        return Err(Error::Handshake(
                            "Could not send zero byte with credentials".to_string(),
                        ));
                    }

                    ret
                }
                MechanismInit => {
                    trace!("Initializing auth mechanisms");
                    self.mechanism_init()?
                }
                WaitingForData | WaitingForOK => {
                    trace!("Waiting for DATA or OK from server");
                    let reply = self.common.read_command()?;
                    match (self.step, reply) {
                        (_, Command::Data(data)) => {
                            trace!("Received DATA from server");
                            let data = data.ok_or_else(|| {
                                Error::Handshake("Received DATA with no data from server".into())
                            })?;
                            self.mechanism_data(data)?
                        }
                        (_, Command::Rejected(_)) => {
                            trace!("Received REJECT from server. Will try next auth mechanism..");
                            self.common.mechanisms.pop_front();
                            self.step = MechanismInit;
                            continue;
                        }
                        (WaitingForOK, Command::Ok(guid)) => {
                            trace!("Received OK from server");
                            match self.server_guid {
                                Some(server_guid) if server_guid != guid => {
                                    return Err(Error::Handshake(format!(
                                        "Server GUID mismatch: expected {server_guid}, got {guid}",
                                    )));
                                }
                                Some(_) => (),
                                None => self.server_guid = Some(guid),
                            }
                            if self.common.socket.read_mut().can_pass_unix_fd() {
                                (WaitingForAgreeUnixFD, Command::NegotiateUnixFD)
                            } else {
                                (Done, Command::Begin)
                            }
                        }
                        (_, reply) => {
                            return Err(Error::Handshake(format!(
                                "Unexpected server AUTH OK reply: {reply}"
                            )));
                        }
                    }
                }
                WaitingForAgreeUnixFD => {
                    trace!("Waiting for Unix FD passing agreement from server");
                    let reply = self.common.read_command()?;
                    match reply {
                        Command::AgreeUnixFD => {
                            trace!("Unix FD passing agreed by server");
                            self.common.cap_unix_fd = true
                        }
                        Command::Error(_) => {
                            trace!("Unix FD passing rejected by server");
                            self.common.cap_unix_fd = false
                        }
                        _ => {
                            return Err(Error::Handshake(format!(
                                "Unexpected server UNIX_FD reply: {reply}"
                            )));
                        }
                    }
                    (Done, Command::Begin)
                }
                Done => {
                    trace!("Handshake done");
                    let (read, write) = self.common.socket.take();
                    return Ok(Authenticated {
                        socket_write: write,
                        socket_read: Some(read),
                        cap_unix_fd: self.common.cap_unix_fd,
                        already_received_bytes: Some(self.common.recv_buffer),
                    });
                }
            };
            self.common.write_command(cmd)?;
            self.step = next_step;
        }
    }
}

/*
 * Server-side handshake logic
 */

/// A representation of an in-progress handshake, server-side

impl fmt::Display for AuthMechanism {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mech = match self {
            AuthMechanism::External => "EXTERNAL",
            AuthMechanism::Cookie => "DBUS_COOKIE_SHA1",
            AuthMechanism::Anonymous => "ANONYMOUS",
        };
        write!(f, "{mech}")
    }
}

impl FromStr for AuthMechanism {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        match s {
            "EXTERNAL" => Ok(AuthMechanism::External),
            "DBUS_COOKIE_SHA1" => Ok(AuthMechanism::Cookie),
            "ANONYMOUS" => Ok(AuthMechanism::Anonymous),
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
                (Some(mech), Some(resp)) => write!(f, "AUTH {mech} {}", hex::encode(resp)),
                (Some(mech), None) => write!(f, "AUTH {mech}"),
                _ => write!(f, "AUTH"),
            },
            Command::Cancel => write!(f, "CANCEL"),
            Command::Begin => write!(f, "BEGIN"),
            Command::Data(data) => match data {
                None => write!(f, "DATA"),
                Some(data) => write!(f, "DATA {}", hex::encode(data)),
            },
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

impl From<hex::FromHexError> for Error {
    fn from(e: hex::FromHexError) -> Self {
        Error::Handshake(format!("Invalid hexcode: {e}"))
    }
}

impl FromStr for Command {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        let mut words = s.split_ascii_whitespace();
        let cmd = match words.next() {
            Some("AUTH") => {
                let mech = if let Some(m) = words.next() {
                    Some(m.parse()?)
                } else {
                    None
                };
                let resp = match words.next() {
                    Some(resp) => Some(hex::decode(resp)?),
                    None => None,
                };
                Command::Auth(mech, resp)
            }
            Some("CANCEL") => Command::Cancel,
            Some("BEGIN") => Command::Begin,
            Some("DATA") => {
                let data = match words.next() {
                    Some(data) => Some(hex::decode(data)?),
                    None => None,
                };

                Command::Data(data)
            }
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
    socket: BoxedSplit,
    recv_buffer: Vec<u8>,
    cap_unix_fd: bool,
    // the current AUTH mechanism is front, ordered by priority
    mechanisms: VecDeque<AuthMechanism>,
}

impl HandshakeCommon {
    /// Start a handshake on this client socket
    pub fn new(socket: BoxedSplit, mechanisms: VecDeque<AuthMechanism>) -> Self {
        Self {
            socket,
            recv_buffer: Vec::new(),
            cap_unix_fd: false,
            mechanisms,
        }
    }

    fn write_command(&mut self, command: Command) -> Result<()> {
        let mut send_buffer = Vec::<u8>::from(command);
        while !send_buffer.is_empty() {
            let written = self.socket.write_mut().sendmsg(&send_buffer, &[])?;
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
            let res = self.socket.read_mut().recvmsg(&mut buf)?;
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

    fn mechanism(&self) -> Result<&AuthMechanism> {
        self.mechanisms
            .front()
            .ok_or_else(|| Error::Handshake("Exhausted available AUTH mechanisms".into()))
    }
}
