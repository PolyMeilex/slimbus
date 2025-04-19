//! D-Bus address handling.
//!
//! Server addresses consist of a transport name followed by a colon, and then an optional,
//! comma-separated list of keys and values in the form key=value.
//!
//! See also:
//!
//! * [Server addresses] in the D-Bus specification.
//!
//! [Server addresses]: https://dbus.freedesktop.org/doc/dbus-specification.html#addresses

pub mod transport;

use crate::{Error, Guid, OwnedGuid, Result};
use std::{collections::HashMap, env, str::FromStr};

use self::transport::Stream;
pub use self::transport::Transport;

/// A bus address
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Address {
    guid: Option<OwnedGuid>,
    transport: Transport,
}

impl Address {
    /// Create a new `Address` from a `Transport`.
    pub fn new(transport: Transport) -> Self {
        Self {
            transport,
            guid: None,
        }
    }

    /// Set the GUID for this address.
    pub fn set_guid<G>(mut self, guid: G) -> Result<Self>
    where
        G: TryInto<OwnedGuid>,
        G::Error: Into<crate::Error>,
    {
        self.guid = Some(guid.try_into().map_err(Into::into)?);

        Ok(self)
    }

    /// The transport details for this address.
    pub fn transport(&self) -> &Transport {
        &self.transport
    }

    pub(crate) fn connect(self) -> Result<Stream> {
        self.transport.connect()
    }

    /// Get the address for session socket respecting the DBUS_SESSION_BUS_ADDRESS environment
    /// variable. If we don't recognize the value (or it's not set) we fall back to
    /// $XDG_RUNTIME_DIR/bus
    pub fn session() -> Result<Self> {
        match env::var("DBUS_SESSION_BUS_ADDRESS") {
            Ok(val) => Self::from_str(&val),
            _ => {
                let id = unsafe { nix::libc::geteuid() }.to_string();
                let runtime_dir =
                    env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| format!("/run/user/{}", id));
                let path = format!("unix:path={runtime_dir}/bus");

                Self::from_str(&path)
            }
        }
    }

    /// Get the address for system bus respecting the DBUS_SYSTEM_BUS_ADDRESS environment
    /// variable. If we don't recognize the value (or it's not set) we fall back to
    /// /var/run/dbus/system_bus_socket
    pub fn system() -> Result<Self> {
        match env::var("DBUS_SYSTEM_BUS_ADDRESS") {
            Ok(val) => Self::from_str(&val),
            _ => Self::from_str("unix:path=/var/run/dbus/system_bus_socket"),
        }
    }

    /// The GUID for this address, if known.
    pub fn guid(&self) -> Option<&Guid<'_>> {
        self.guid.as_ref().map(|guid| guid.inner())
    }
}

impl FromStr for Address {
    type Err = Error;

    /// Parse the transport part of a D-Bus address into a `Transport`.
    fn from_str(address: &str) -> Result<Self> {
        let col = address
            .find(':')
            .ok_or_else(|| Error::Address("address has no colon".to_owned()))?;
        let transport = &address[..col];
        let mut options = HashMap::new();

        if address.len() > col + 1 {
            for kv in address[col + 1..].split(',') {
                let (k, v) = match kv.find('=') {
                    Some(eq) => (&kv[..eq], &kv[eq + 1..]),
                    None => {
                        return Err(Error::Address(
                            "missing = when parsing key/value".to_owned(),
                        ))
                    }
                };
                if options.insert(k, v).is_some() {
                    return Err(Error::Address(format!(
                        "Key `{k}` specified multiple times"
                    )));
                }
            }
        }

        Ok(Self {
            guid: options
                .remove("guid")
                .map(|s| Guid::from_str(s).map(|guid| OwnedGuid::from(guid).to_owned()))
                .transpose()?,
            transport: Transport::from_options(transport, options)?,
        })
    }
}

impl TryFrom<&str> for Address {
    type Error = Error;

    fn try_from(value: &str) -> Result<Self> {
        Self::from_str(value)
    }
}

impl From<Transport> for Address {
    fn from(transport: Transport) -> Self {
        Self::new(transport)
    }
}
