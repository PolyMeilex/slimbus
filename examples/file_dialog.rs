#![allow(clippy::single_match)]

use std::{num::NonZeroU32, os::fd::AsRawFd};

use slimbus::names::UniqueName;
use slimbus::zvariant::{DeserializeDict, SerializeDict, Type};
use slimbus::{names::OwnedUniqueName, Connection, Message, Result, SocketReader};
use zvariant::OwnedObjectPath;

#[derive(serde::Serialize, Type, Debug)]
pub struct HandleToken(String);

impl Default for HandleToken {
    fn default() -> Self {
        use rand::{distr::Alphanumeric, thread_rng, Rng};

        let token: String = thread_rng()
            .sample_iter(Alphanumeric)
            .take(10)
            .map(char::from)
            .collect();

        Self(format!("rfd_{token}"))
    }
}

#[derive(SerializeDict, Type, Debug, Default)]
#[zvariant(signature = "dict")]
struct OpenFileOptions {
    handle_token: HandleToken,
    accept_label: Option<String>,
    modal: Option<bool>,
    multiple: Option<bool>,
    directory: Option<bool>,
    // filters: Vec<Filter>,
    // current_filter: Option<Filter>,
    // choices: Vec<Choice>,
    // current_folder: Option<CString>,
}

#[derive(Debug, Type, DeserializeDict)]
#[zvariant(signature = "dict")]
pub struct SelectedFiles {
    uris: Vec<String>,
}

#[derive(Debug, Type, serde::Deserialize)]
pub struct Response {
    response: u32,
    results: SelectedFiles,
}

fn hello(connection: &mut Connection, reader: &mut SocketReader) -> Result<OwnedUniqueName> {
    let msg = Message::method("/org/freedesktop/DBus", "Hello")?
        .destination("org.freedesktop.DBus")?
        .interface("org.freedesktop.DBus")?
        .build(&())?;

    connection.send(&msg)?;

    let serial = msg.primary_header().serial_num();
    let res = wait_for_response(reader, serial);
    let body = res.body().deserialize::<UniqueName>()?.to_owned();
    Ok(body)
}

fn open_file(
    connection: &mut Connection,
    reader: &mut SocketReader,
    unique_name: &OwnedUniqueName,
) -> Result<String> {
    let opts = &OpenFileOptions::default();

    let our_obj_path = {
        let handle_token = &opts.handle_token.0;
        let unique_identifier = unique_name.trim_start_matches(':').replace('.', "_");
        let obj_path =
            format!("/org/freedesktop/portal/desktop/request/{unique_identifier}/{handle_token}");
        add_response_signal_match(connection, obj_path.as_str())?;
        obj_path
    };

    let mut builder = Message::method("/org/freedesktop/portal/desktop", "OpenFile")?;
    builder = builder.destination("org.freedesktop.portal.Desktop")?;
    builder = builder.interface("org.freedesktop.portal.FileChooser")?;
    let msg = builder.build(&("", "Title", opts))?;

    connection.send(&msg)?;

    let serial = msg.primary_header().serial_num();
    let res = wait_for_response(reader, serial);
    let obj_path = res.body();
    let obj_path: OwnedObjectPath = obj_path.deserialize()?;

    // Check for pre 0.9 xdp version
    if our_obj_path != obj_path.as_str() {
        add_response_signal_match(connection, obj_path.as_str())?;
        Ok(obj_path.to_string())
    } else {
        Ok(our_obj_path)
    }
}

fn add_match(connection: &mut Connection, params: &[&str]) -> Result<()> {
    let mut builder = Message::method("/org/freedesktop/DBus", "AddMatch")?;
    builder = builder.destination("org.freedesktop.DBus")?;
    builder = builder.interface("org.freedesktop.DBus")?;

    let params = params.join(",");
    let msg = builder.build(&params)?;

    connection.send(&msg)?;

    Ok(())
}

fn add_response_signal_match(connection: &mut Connection, obj_path: &str) -> Result<()> {
    add_match(
        connection,
        &[
            "type='signal'",
            "sender='org.freedesktop.portal.Desktop'",
            &format!("path='{}'", obj_path),
            "interface='org.freedesktop.portal.Request'",
            "member='Response'",
        ],
    )?;
    Ok(())
}

fn main() -> Result<()> {
    let (mut connection, mut reader) = Connection::session()?;
    slimbus::set_blocking(connection.as_raw_fd(), true);

    let unique_name = hello(&mut connection, &mut reader)?;
    let obj_path = open_file(&mut connection, &mut reader, &unique_name)?;

    let response: Response = loop {
        let msg = reader.read_socket().unwrap();
        println!("<- {:?}", msg);

        let header = msg.header();

        let Some(interface) = header.interface() else {
            continue;
        };
        let Some(member) = header.member() else {
            continue;
        };
        let Some(path) = header.path() else {
            continue;
        };
        if path.as_str() != obj_path {
            continue;
        }

        match (interface.as_str(), member.as_str()) {
            ("org.freedesktop.portal.Request", "Response") => {
                let body = msg.body();
                break body.deserialize()?;
            }
            _ => {}
        }
    };

    dbg!(response.response);
    dbg!(response.results.uris);

    Ok(())
}

fn wait_for_response(reader: &mut SocketReader, serial: NonZeroU32) -> Message {
    loop {
        let msg = reader.read_socket().unwrap();
        println!("<- {:?}", msg);
        if msg.header().reply_serial() == Some(serial) {
            break msg;
        }
    }
}
