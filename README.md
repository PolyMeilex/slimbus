[zbus](https://github.com/dbus2/zbus/tree/main) but on a diet (2200 LOC)

This is basically stripped down alternative to zbus, it's only goal is to be as small as possible. No heavy deps, no async, no Windows support, no macOS support, no fancy abstractions, just a socket and message de/serialization.

Current dependency graph (hopefully it will get even smaller):
```
slimbus
├── enumflags2 v0.7
├── log v0.4
├── nix v0.29
├── serde v1.0
└── zvariant v5.4
```

```rs
fn main() -> Result<()> {
    let (mut connection, mut reader) = Connection::session()?;

    let msg = Message::method("/org/freedesktop/DBus", "Hello")?
        .destination("org.freedesktop.DBus")?
        .interface("org.freedesktop.DBus")?
        .build(&())?;

    let serial = msg.primary_header().serial_num();
    let name = loop {
        let msg = reader.read_socket()?;

        println!("Got message: {:?}", msg);
        if msg.header().reply_serial() == Some(serial) {
            let body: OwnedUniqueName = msg.body().deserialize()?;
            break body;
        }
    };

    dbg!(name);
}
```
