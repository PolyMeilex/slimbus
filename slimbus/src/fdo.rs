//! D-Bus standard interfaces.
//!
//! The D-Bus specification defines the message bus messages and some standard interfaces that may
//! be useful across various D-Bus applications. This module provides their proxy.

use zvariant::{DeserializeDict, SerializeDict, Type};

/// Credentials of a process connected to a bus server.
///
/// If unable to determine certain credentials (for instance, because the process is not on the same
/// machine as the bus daemon, or because this version of the bus daemon does not support a
/// particular security framework), or if the values of those credentials cannot be represented as
/// documented here, then those credentials are omitted.
///
/// **Note**: unknown keys, in particular those with "." that are not from the specification, will
/// be ignored. Use your own implementation or contribute your keys here, or in the specification.
#[derive(Debug, Default, DeserializeDict, PartialEq, Eq, SerializeDict, Type)]
#[zvariant(signature = "a{sv}")]
pub struct ConnectionCredentials {
    #[zvariant(rename = "UnixUserID")]
    pub(crate) unix_user_id: Option<u32>,

    #[zvariant(rename = "UnixGroupIDs")]
    pub(crate) unix_group_ids: Option<Vec<u32>>,

    #[zvariant(rename = "ProcessID")]
    pub(crate) process_id: Option<u32>,

    #[zvariant(rename = "WindowsSID")]
    pub(crate) windows_sid: Option<String>,

    #[zvariant(rename = "LinuxSecurityLabel")]
    pub(crate) linux_security_label: Option<Vec<u8>>,
}

impl ConnectionCredentials {
    /// The numeric Unix user ID, as defined by POSIX.
    pub fn unix_user_id(&self) -> Option<u32> {
        self.unix_user_id
    }

    /// The numeric Unix group IDs (including both the primary group and the supplementary groups),
    /// as defined by POSIX, in numerically sorted order. This array is either complete or absent:
    /// if the message bus is able to determine some but not all of the caller's groups, or if one
    /// of the groups is not representable in a UINT32, it must not add this credential to the
    /// dictionary.
    pub fn unix_group_ids(&self) -> Option<&Vec<u32>> {
        self.unix_group_ids.as_ref()
    }

    /// Same as [`ConnectionCredentials::unix_group_ids`], but consumes `self` and returns the group
    /// IDs Vec.
    pub fn into_unix_group_ids(self) -> Option<Vec<u32>> {
        self.unix_group_ids
    }

    /// The numeric process ID, on platforms that have this concept. On Unix, this is the process ID
    /// defined by POSIX.
    pub fn process_id(&self) -> Option<u32> {
        self.process_id
    }

    /// The Windows security identifier in its string form, e.g.
    /// `S-1-5-21-3623811015-3361044348-30300820-1013` for a domain or local computer user or
    /// "S-1-5-18` for the LOCAL_SYSTEM user.
    pub fn windows_sid(&self) -> Option<&String> {
        self.windows_sid.as_ref()
    }

    /// Same as [`ConnectionCredentials::windows_sid`], but consumes `self` and returns the SID
    /// string.
    pub fn into_windows_sid(self) -> Option<String> {
        self.windows_sid
    }

    /// On Linux systems, the security label that would result from the SO_PEERSEC getsockopt call.
    /// The array contains the non-zero bytes of the security label in an unspecified
    /// ASCII-compatible encoding, followed by a single zero byte.
    ///
    /// For example, the SELinux context `system_u:system_r:init_t:s0` (a string of length 27) would
    /// be encoded as 28 bytes ending with `':', 's', '0', '\x00'`
    ///
    /// On SELinux systems this is the SELinux context, as output by `ps -Z` or `ls -Z`. Typical
    /// values might include `system_u:system_r:init_t:s0`,
    /// `unconfined_u:unconfined_r:unconfined_t:s0-s0:c0.c1023`, or
    /// `unconfined_u:unconfined_r:chrome_sandbox_t:s0-s0:c0.c1023`.
    ///
    /// On Smack systems, this is the Smack label. Typical values might include `_`, `*`, `User`,
    /// `System` or `System::Shared`.
    ///
    /// On AppArmor systems, this is the AppArmor context, a composite string encoding the AppArmor
    /// label (one or more profiles) and the enforcement mode. Typical values might include
    /// `unconfined`, `/usr/bin/firefox (enforce)` or `user1 (complain)`.
    pub fn linux_security_label(&self) -> Option<&Vec<u8>> {
        self.linux_security_label.as_ref()
    }

    /// Same as [`ConnectionCredentials::linux_security_label`], but consumes `self` and returns
    /// the security label bytes.
    pub fn into_linux_security_label(self) -> Option<Vec<u8>> {
        self.linux_security_label
    }

    /// Set the numeric Unix user ID, as defined by POSIX.
    pub fn set_unix_user_id(mut self, unix_user_id: u32) -> Self {
        self.unix_user_id = Some(unix_user_id);

        self
    }

    /// Add a numeric Unix group ID.
    ///
    /// See [`ConnectionCredentials::unix_group_ids`] for more information.
    pub fn add_unix_group_id(mut self, unix_group_id: u32) -> Self {
        self.unix_group_ids
            .get_or_insert_with(Vec::new)
            .push(unix_group_id);

        self
    }

    /// Set the numeric process ID, on platforms that have this concept.
    ///
    /// See [`ConnectionCredentials::process_id`] for more information.
    pub fn set_process_id(mut self, process_id: u32) -> Self {
        self.process_id = Some(process_id);

        self
    }

    /// Set the Windows security identifier in its string form.
    pub fn set_windows_sid(mut self, windows_sid: String) -> Self {
        self.windows_sid = Some(windows_sid);

        self
    }

    /// Set the Linux security label.
    ///
    /// See [`ConnectionCredentials::linux_security_label`] for more information.
    pub fn set_linux_security_label(mut self, linux_security_label: Vec<u8>) -> Self {
        self.linux_security_label = Some(linux_security_label);

        self
    }
}

/// Errors from <https://gitlab.freedesktop.org/dbus/dbus/-/blob/master/dbus/dbus-protocol.h>
#[derive(Clone, Debug, PartialEq)]
#[allow(clippy::upper_case_acronyms)]
pub enum Error {
    /// Unknown or fall-through ZBus error.
    ZBus(zbus::Error),

    /// A generic error; "something went wrong" - see the error message for more.
    Failed(String),

    /// There was not enough memory to complete an operation.
    NoMemory(String),

    /// The bus doesn't know how to launch a service to supply the bus name you wanted.
    ServiceUnknown(String),

    /// The bus name you referenced doesn't exist (i.e. no application owns it).
    NameHasNoOwner(String),

    /// No reply to a message expecting one, usually means a timeout occurred.
    NoReply(String),

    /// Something went wrong reading or writing to a socket, for example.
    IOError(String),

    /// A D-Bus bus address was malformed.
    BadAddress(String),

    /// Requested operation isn't supported (like ENOSYS on UNIX).
    NotSupported(String),

    /// Some limited resource is exhausted.
    LimitsExceeded(String),

    /// Security restrictions don't allow doing what you're trying to do.
    AccessDenied(String),

    /// Authentication didn't work.
    AuthFailed(String),

    /// Unable to connect to server (probably caused by ECONNREFUSED on a socket).
    NoServer(String),

    /// Certain timeout errors, possibly ETIMEDOUT on a socket.
    /// Note that `TimedOut` is used for message reply timeouts.
    Timeout(String),

    /// No network access (probably ENETUNREACH on a socket).
    NoNetwork(String),

    /// Can't bind a socket since its address is in use (i.e. EADDRINUSE).
    AddressInUse(String),

    /// The connection is disconnected and you're trying to use it.
    Disconnected(String),

    /// Invalid arguments passed to a method call.
    InvalidArgs(String),

    /// Missing file.
    FileNotFound(String),

    /// Existing file and the operation you're using does not silently overwrite.
    FileExists(String),

    /// Method name you invoked isn't known by the object you invoked it on.
    UnknownMethod(String),

    /// Object you invoked a method on isn't known.
    UnknownObject(String),

    /// Interface you invoked a method on isn't known by the object.
    UnknownInterface(String),

    /// Property you tried to access isn't known by the object.
    UnknownProperty(String),

    /// Property you tried to set is read-only.
    PropertyReadOnly(String),

    /// Certain timeout errors, e.g. while starting a service.
    TimedOut(String),

    /// Tried to remove or modify a match rule that didn't exist.
    MatchRuleNotFound(String),

    /// The match rule isn't syntactically valid.
    MatchRuleInvalid(String),

    /// Tried to get a UNIX process ID and it wasn't available.
    UnixProcessIdUnknown(String),

    /// A type signature is not valid.
    InvalidSignature(String),

    /// A file contains invalid syntax or is otherwise broken.
    InvalidFileContent(String),

    /// Asked for SELinux security context and it wasn't available.
    SELinuxSecurityContextUnknown(String),

    /// Asked for ADT audit data and it wasn't available.
    AdtAuditDataUnknown(String),

    /// There's already an object with the requested object path.
    ObjectPathInUse(String),

    /// The message meta data does not match the payload. e.g. expected number of file descriptors
    /// were not sent over the socket this message was received on.
    InconsistentMessage(String),

    /// The message is not allowed without performing interactive authorization, but could have
    /// succeeded if an interactive authorization step was allowed.
    InteractiveAuthorizationRequired(String),

    /// The connection is not from a container, or the specified container instance does not exist.
    NotContainer(String),
}

impl std::error::Error for Error {}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self)
    }
}

/// Alias for a `Result` with the error type [`zbus::fdo::Error`].
///
/// [`zbus::fdo::Error`]: enum.Error.html
pub type Result<T> = std::result::Result<T, Error>;
