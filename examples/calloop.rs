#![allow(clippy::single_match)]

use std::{
    num::NonZeroU32,
    os::fd::{AsRawFd, BorrowedFd},
};

use calloop::{generic::Generic, EventLoop, Interest};
use slimbus::{names::OwnedUniqueName, zvariant::OwnedValue, Connection, Message};

enum State {
    Hello(NonZeroU32),
    Listening,
}

struct App {
    connection: Connection,
    state: State,
}

impl App {
    fn new(connection: Connection, hello_serial: NonZeroU32) -> Self {
        Self {
            connection,
            state: State::Hello(hello_serial),
        }
    }

    fn set_up_signals(&mut self) -> Result<(), Box<dyn std::error::Error>> {
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
        self.connection.send(&msg)?;
        self.state = State::Listening;

        Ok(())
    }

    fn handle_message(&mut self, msg: Message) -> Result<(), Box<dyn std::error::Error>> {
        println!("Got message: {:?}", msg);

        match self.state {
            State::Hello(serial) => {
                if msg.header().reply_serial() == Some(serial) {
                    let name: OwnedUniqueName = msg.body().deserialize().unwrap();
                    dbg!(name);
                    self.set_up_signals()?;
                }
            }
            State::Listening => {
                let header = msg.header();

                let Some(interface) = header.interface() else {
                    return Ok(());
                };
                let Some(member) = header.member() else {
                    return Ok(());
                };

                match (interface.as_str(), member.as_str()) {
                    ("org.freedesktop.portal.Settings", "SettingChanged") => {
                        let body: (String, String, OwnedValue) = msg.body().deserialize()?;
                        dbg!(body);
                    }
                    _ => {}
                }
            }
        }

        Ok(())
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut event_loop = EventLoop::<App>::try_new()?;

    let (mut connection, mut reader) = Connection::session()?;

    slimbus::set_blocking(connection.as_raw_fd(), false);

    let fd = connection.as_raw_fd();
    let fd = unsafe { BorrowedFd::borrow_raw(fd) };
    event_loop
        .handle()
        .insert_source(
            Generic::new(fd, Interest::READ, calloop::Mode::Level),
            move |_, _, app| {
                let msg = reader.read_socket().unwrap();
                app.handle_message(msg).unwrap();
                Ok(calloop::PostAction::Continue)
            },
        )
        .unwrap();

    let msg = Message::method("/org/freedesktop/DBus", "Hello")?
        .destination("org.freedesktop.DBus")?
        .interface("org.freedesktop.DBus")?
        .build(&())?;

    connection.send(&msg)?;

    let serial = msg.primary_header().serial_num();
    let mut app = App::new(connection, serial);

    event_loop.run(None, &mut app, |_| {}).unwrap();

    Ok(())
}
