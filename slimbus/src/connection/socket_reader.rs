use zvariant::{
    serialized::{self, Context},
    Endian,
};

use crate::{
    message::header::{PrimaryHeader, MAX_MESSAGE_SIZE, MIN_MESSAGE_SIZE},
    padding_for_8_bytes, Message,
};

use super::socket::UnixStreamRead;

#[derive(Debug)]
pub struct SocketReader {
    socket: UnixStreamRead,
    already_received_bytes: Option<Vec<u8>>,
    prev_seq: u64,
}

impl SocketReader {
    pub fn new(socket: UnixStreamRead, already_received_bytes: Vec<u8>) -> Self {
        Self {
            socket,
            already_received_bytes: Some(already_received_bytes),
            prev_seq: 0,
        }
    }

    pub fn read_socket(&mut self) -> crate::Result<Message> {
        let mut bytes = self
            .already_received_bytes
            .take()
            .unwrap_or_else(|| Vec::with_capacity(MIN_MESSAGE_SIZE));
        let mut pos = bytes.len();
        let mut fds = vec![];
        if pos < MIN_MESSAGE_SIZE {
            bytes.resize(MIN_MESSAGE_SIZE, 0);
            // We don't have enough data to make a proper message header yet.
            // Some partial read may be in raw_in_buffer, so we try to complete it
            // until we have MIN_MESSAGE_SIZE bytes
            //
            // Given that MIN_MESSAGE_SIZE is 16, this codepath is actually extremely unlikely
            // to be taken more than once
            while pos < MIN_MESSAGE_SIZE {
                let res = self.socket.recvmsg(&mut bytes[pos..])?;
                let len = {
                    fds.extend(res.1);
                    res.0
                };
                pos += len;
                if len == 0 {
                    return Err(crate::Error::InputOutput(
                        std::io::Error::new(
                            std::io::ErrorKind::UnexpectedEof,
                            "failed to receive message",
                        )
                        .into(),
                    ));
                }
            }
        }

        let (primary_header, fields_len) = PrimaryHeader::read(&bytes)?;
        let header_len = MIN_MESSAGE_SIZE + fields_len as usize;
        let body_padding = padding_for_8_bytes(header_len);
        let body_len = primary_header.body_len() as usize;
        let total_len = header_len + body_padding + body_len;
        if total_len > MAX_MESSAGE_SIZE {
            return Err(crate::Error::ExcessData);
        }

        // By this point we have a full primary header, so we know the exact length of the complete
        // message.
        bytes.resize(total_len, 0);

        // Now we have an incomplete message; read the rest
        while pos < total_len {
            let res = self.socket.recvmsg(&mut bytes[pos..])?;
            let read = {
                fds.extend(res.1);
                res.0
            };
            pos += read;
            if read == 0 {
                return Err(crate::Error::InputOutput(
                    std::io::Error::new(
                        std::io::ErrorKind::UnexpectedEof,
                        "failed to receive message",
                    )
                    .into(),
                ));
            }
        }

        // If we reach here, the message is complete; return it
        let seq = self.prev_seq + 1;
        self.prev_seq = seq;
        let endian = Endian::from(primary_header.endian_sig());
        let ctxt = Context::new_dbus(endian, 0);
        let bytes = serialized::Data::new_fds(bytes, ctxt, fds);
        Message::from_raw_parts(bytes, seq)
    }
}
