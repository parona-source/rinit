use std::{
    io::{
        self,
        Read,
        Write,
    },
    os::unix::net::UnixStream,
};

use snafu::{
    ResultExt,
    Snafu,
};

use crate::Request;

#[derive(Debug, Snafu)]
pub enum Error {
    #[snafu(display("connection to {socket} failed"))]
    ConnectionFailed { socket: String, source: io::Error },
    #[snafu(display("failed to read request"))]
    ReadFailed { source: io::Error },
    #[snafu(display("failed to write request"))]
    WriteFailed { source: io::Error },
}

pub struct Connection {
    stream: UnixStream,
}

impl Connection {
    pub fn new(socket: &str) -> Result<Self, Error> {
        let stream =
            UnixStream::connect(socket).with_context(|_| ConnectionFailedSnafu { socket })?;
        Ok(Self { stream })
    }

    pub fn new_host_address() -> Result<Self, Error> {
        Self::new(crate::get_host_address())
    }

    pub fn send(
        &mut self,
        buf: &[u8],
    ) -> Result<(), Error> {
        self.stream
            .write_all(buf)
            .with_context(|_| WriteFailedSnafu {})?;
        self.stream
            .write("\n".as_bytes())
            .with_context(|_| WriteFailedSnafu {})?;

        Ok(())
    }

    pub fn send_request(
        &mut self,
        request: Request,
    ) -> Result<(), Error> {
        self.send(&serde_json::to_vec(&request).unwrap())
    }

    pub fn recv(&mut self) -> Result<String, Error> {
        let mut s = String::new();
        self.stream
            .read_to_string(&mut s)
            .with_context(|_| ReadFailedSnafu {})?;

        Ok(s)
    }
}
