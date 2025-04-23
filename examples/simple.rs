use std::os::fd::AsRawFd;

use slimbus::{names::UniqueName, zvariant::OwnedValue, Connection, Message, Result};

const INTERFACE: &str = "org.freedesktop.portal.Settings";
const DESTINATION: &str = "org.freedesktop.portal.Desktop";
const PATH: &str = "/org/freedesktop/portal/desktop";

fn main() -> Result<()> {
    let (mut connection, mut reader) = Connection::session()?;

    slimbus::set_blocking(connection.as_raw_fd(), false);

    {
        let msg = Message::method("/org/freedesktop/DBus", "Hello")?
            .destination("org.freedesktop.DBus")?
            .interface("org.freedesktop.DBus")?
            .build(&())?;

        connection.send(&msg)?;

        let serial = msg.primary_header().serial_num();
        let name = loop {
            slimbus::poll(connection.as_raw_fd(), -1);

            let msg = reader.read_socket().unwrap();
            println!("Got message: {:?}", msg);
            if msg.header().reply_serial() == Some(serial) {
                let body = msg.body().deserialize::<UniqueName>()?.to_owned();
                break body;
            }
        };
        dbg!(name);
    }
    //
    // {
    //     let mut builder = Message::method(PATH, "Read")?;
    //     builder = builder.destination(DESTINATION)?;
    //     builder = builder.interface(INTERFACE)?;
    //     let msg = builder.build(&("org.freedesktop.appearance", "color-scheme"))?;
    //
    //     let serial = msg.primary_header().serial_num();
    //
    //     connection.send(&msg)?;
    //
    //     loop {
    //         let msg = reader.read_socket().unwrap();
    //         if msg.header().reply_serial() == Some(serial) {
    //             let body: OwnedValue = msg.body().deserialize()?;
    //             dbg!(body);
    //             break;
    //         }
    //     }
    // }
    {
        let mut builder = Message::method("/org/freedesktop/DBus", "AddMatch")?;
        builder = builder.destination("org.freedesktop.DBus")?;
        builder = builder.interface("org.freedesktop.DBus")?;

        let params = [
            "type='signal'",
            "sender='org.freedesktop.portal.Desktop'",
            "path='/org/freedesktop/portal/desktop'",
            "interface='org.freedesktop.portal.Settings'",
            "member='SettingChanged'",
            "arg0='org.freedesktop.appearance'",
            "arg1='color-scheme'",
        ]
        .join(",");

        let msg = builder.build(&params)?;

        let serial = msg.primary_header().serial_num();

        connection.send(&msg)?;

        loop {
            slimbus::poll(connection.as_raw_fd(), -1);

            let msg = reader.read_socket().unwrap();
            println!("Got message: {:?}", msg);
            if msg.header().reply_serial() == Some(serial) {
                // let body: OwnedValue = msg.body().deserialize()?;
                // dbg!(body);
                break;
            }
        }
    }

    loop {
        slimbus::poll(connection.as_raw_fd(), -1);

        let msg = reader.read_socket().unwrap();
        println!("Got message: {:?}", msg);

        let header = msg.header();

        let Some(interface) = header.interface() else {
            continue;
        };
        let Some(member) = header.member() else {
            continue;
        };

        match (interface.as_str(), member.as_str()) {
            ("org.freedesktop.portal.Settings", "SettingChanged") => {
                let body: (String, String, OwnedValue) = msg.body().deserialize()?;
                dbg!(body);
            }
            _ => {}
        }
    }

    // Ok(())
}
