//! A hand-written WebSocket client (RFC 6455): the HTTP upgrade, frames out
//! always masked, frames in with fragmentation, ping answered by pong, and the
//! close handshake. No WebSocket crate: this is decision 3 of the voice design
//! under test, so it stands on what footplate's api.rs already has (rustls with
//! ring and the system's roots, std's TcpStream) and adds no dependency.
//!
//! The pieces are plain and tested on their own against the RFC's byte
//! examples (`Decoder`, `Assembler`, `encode_frame`, `sha1`); `Conn` is the one
//! that touches a socket.

use std::fmt;
use std::io::{self, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

/// The string RFC 6455 has the accept key made with.
pub const GUID: &str = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";
/// A message larger than this is refused: a frame's length is the server's
/// word, and nothing here should allocate on it unchecked.
const MAX_MESSAGE: usize = 64 << 20;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const HEADER_LIMIT: usize = 16 << 10;

#[derive(Debug)]
pub enum Error {
    Io(io::Error),
    /// The server answered the upgrade with something else: its status line and
    /// what it said, with anything that looks like a key taken out.
    Refused(String),
    /// A frame or handshake that breaks the RFC.
    Protocol(String),
    /// The connection ended without a close frame.
    Closed,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io(err) => write!(f, "{err}"),
            Error::Refused(why) => write!(f, "the server refused the upgrade: {why}"),
            Error::Protocol(why) => write!(f, "websocket protocol error: {why}"),
            Error::Closed => write!(f, "the connection ended without a close frame"),
        }
    }
}

impl From<io::Error> for Error {
    fn from(err: io::Error) -> Error {
        Error::Io(err)
    }
}

fn protocol(why: impl Into<String>) -> Error {
    Error::Protocol(why.into())
}

/// `ws://host[:port]/path` or `wss://...`.
#[derive(Clone, Debug, PartialEq)]
pub struct Url {
    pub tls: bool,
    pub host: String,
    pub port: u16,
    pub path: String,
}

impl Url {
    pub fn parse(text: &str) -> Result<Url, Error> {
        let (tls, rest) = if let Some(rest) = text.strip_prefix("wss://") {
            (true, rest)
        } else if let Some(rest) = text.strip_prefix("ws://") {
            (false, rest)
        } else {
            return Err(protocol(format!(
                "{text:?} is not a ws:// or wss:// address"
            )));
        };
        let (authority, path) = match rest.find('/') {
            Some(at) => (&rest[..at], &rest[at..]),
            None => (rest, "/"),
        };
        let default = if tls { 443 } else { 80 };
        let (host, port) = match authority.rsplit_once(':') {
            Some((host, port)) if !host.ends_with(']') || host.starts_with('[') => {
                let port = port
                    .parse()
                    .map_err(|_| protocol(format!("{port:?} is not a port")))?;
                (host.trim_matches(['[', ']']), port)
            }
            _ => (authority, default),
        };
        if host.is_empty() {
            return Err(protocol("the address names no host"));
        }
        Ok(Url {
            tls,
            host: host.to_string(),
            port,
            path: path.to_string(),
        })
    }

    fn authority(&self) -> String {
        let default = if self.tls { 443 } else { 80 };
        if self.port == default {
            self.host.clone()
        } else {
            format!("{}:{}", self.host, self.port)
        }
    }
}

// ---- the frame codec -------------------------------------------------------

pub const CONTINUATION: u8 = 0x0;
pub const TEXT: u8 = 0x1;
#[cfg(test)]
pub const BINARY: u8 = 0x2;
pub const CLOSE: u8 = 0x8;
pub const PING: u8 = 0x9;
pub const PONG: u8 = 0xA;

/// One frame as it stands on the wire, unmasked.
#[derive(Debug, PartialEq)]
pub struct Frame {
    pub fin: bool,
    pub opcode: u8,
    pub payload: Vec<u8>,
}

/// A frame's bytes. A client masks every frame it sends (`mask` is `Some`), a
/// server none; the key is the caller's, since where it comes from is not the
/// codec's business.
pub fn encode_frame(fin: bool, opcode: u8, payload: &[u8], mask: Option<[u8; 4]>) -> Vec<u8> {
    let mut out = Vec::with_capacity(payload.len() + 14);
    out.push(if fin { 0x80 } else { 0 } | opcode);
    let masked = if mask.is_some() { 0x80 } else { 0 };
    match payload.len() {
        len if len < 126 => out.push(masked | len as u8),
        len if len <= 0xFFFF => {
            out.push(masked | 126);
            out.extend_from_slice(&(len as u16).to_be_bytes());
        }
        len => {
            out.push(masked | 127);
            out.extend_from_slice(&(len as u64).to_be_bytes());
        }
    }
    match mask {
        Some(key) => {
            out.extend_from_slice(&key);
            out.extend(payload.iter().enumerate().map(|(i, b)| b ^ key[i % 4]));
        }
        None => out.extend_from_slice(payload),
    }
    out
}

/// Frames out of the bytes as they arrive, however they are cut.
pub struct Decoder {
    buf: Vec<u8>,
    /// A client expects every frame unmasked; a server (the stand-in) every
    /// frame masked. The RFC has either side close on the other.
    expect_masked: bool,
}

impl Decoder {
    pub fn new(expect_masked: bool) -> Decoder {
        Decoder {
            buf: Vec::new(),
            expect_masked,
        }
    }

    pub fn push(&mut self, bytes: &[u8]) {
        self.buf.extend_from_slice(bytes);
    }

    /// The next whole frame, or `None` while its bytes are not all here.
    pub fn next_frame(&mut self) -> Result<Option<Frame>, Error> {
        let buf = &self.buf;
        if buf.len() < 2 {
            return Ok(None);
        }
        let (b0, b1) = (buf[0], buf[1]);
        if b0 & 0x70 != 0 {
            return Err(protocol(
                "a reserved bit is set (no extension was asked for)",
            ));
        }
        let fin = b0 & 0x80 != 0;
        let opcode = b0 & 0x0F;
        if !matches!(opcode, 0..=2 | 8..=10) {
            return Err(protocol(format!("opcode {opcode} is reserved")));
        }
        let masked = b1 & 0x80 != 0;
        if masked != self.expect_masked {
            return Err(protocol(if masked {
                "the server sent a masked frame"
            } else {
                "the client sent an unmasked frame"
            }));
        }
        let (len, mut at) = match b1 & 0x7F {
            126 => {
                if buf.len() < 4 {
                    return Ok(None);
                }
                (u16::from_be_bytes([buf[2], buf[3]]) as u64, 4)
            }
            127 => {
                if buf.len() < 10 {
                    return Ok(None);
                }
                let mut eight = [0u8; 8];
                eight.copy_from_slice(&buf[2..10]);
                (u64::from_be_bytes(eight), 10)
            }
            short => (short as u64, 2),
        };
        if len >> 63 != 0 {
            return Err(protocol("a frame's length has its top bit set"));
        }
        if opcode >= CLOSE && (len > 125 || !fin) {
            return Err(protocol(
                "a control frame is longer than 125 bytes or fragmented",
            ));
        }
        if len > MAX_MESSAGE as u64 {
            return Err(protocol(format!(
                "a frame of {len} bytes is over the limit"
            )));
        }
        let len = len as usize;
        let key = if masked {
            if buf.len() < at + 4 {
                return Ok(None);
            }
            let key = [buf[at], buf[at + 1], buf[at + 2], buf[at + 3]];
            at += 4;
            Some(key)
        } else {
            None
        };
        if buf.len() < at + len {
            return Ok(None);
        }
        let mut payload = buf[at..at + len].to_vec();
        if let Some(key) = key {
            for (i, byte) in payload.iter_mut().enumerate() {
                *byte ^= key[i % 4];
            }
        }
        self.buf.drain(..at + len);
        Ok(Some(Frame {
            fin,
            opcode,
            payload,
        }))
    }
}

/// What the far end sent, as whole messages: data in one piece, control
/// frames as they came (they may stand between the fragments of a message).
#[derive(Debug, PartialEq)]
pub enum Incoming {
    Text(String),
    Binary(Vec<u8>),
    Ping(Vec<u8>),
    Pong(Vec<u8>),
    Close(Option<u16>, String),
}

/// Adds fragment data without letting the assembled message exceed its cap.
fn extend_bounded(data: &mut Vec<u8>, bytes: &[u8], limit: usize) -> Result<(), Error> {
    if data.len() > limit || bytes.len() > limit - data.len() {
        return Err(protocol("a message is over the limit"));
    }
    data.extend_from_slice(bytes);
    Ok(())
}

/// Joins fragments into messages and checks the order the RFC asks for.
#[derive(Default)]
pub struct Assembler {
    opcode: Option<u8>,
    data: Vec<u8>,
}

impl Assembler {
    pub fn accept(&mut self, frame: Frame) -> Result<Option<Incoming>, Error> {
        match frame.opcode {
            PING => Ok(Some(Incoming::Ping(frame.payload))),
            PONG => Ok(Some(Incoming::Pong(frame.payload))),
            CLOSE => match frame.payload.len() {
                0 => Ok(Some(Incoming::Close(None, String::new()))),
                1 => Err(protocol("a close frame has a one-byte body")),
                _ => {
                    let code = u16::from_be_bytes([frame.payload[0], frame.payload[1]]);
                    let reason = String::from_utf8(frame.payload[2..].to_vec())
                        .map_err(|_| protocol("a close reason is not UTF-8"))?;
                    Ok(Some(Incoming::Close(Some(code), reason)))
                }
            },
            CONTINUATION => {
                if self.opcode.is_none() {
                    return Err(protocol("a continuation frame with no message begun"));
                }
                extend_bounded(&mut self.data, &frame.payload, MAX_MESSAGE)?;
                self.finish_if(frame.fin)
            }
            opcode => {
                if self.opcode.is_some() {
                    return Err(protocol("a new message begun inside another"));
                }
                self.opcode = Some(opcode);
                self.data.clear();
                extend_bounded(&mut self.data, &frame.payload, MAX_MESSAGE)?;
                self.finish_if(frame.fin)
            }
        }
    }

    fn finish_if(&mut self, fin: bool) -> Result<Option<Incoming>, Error> {
        if self.data.len() > MAX_MESSAGE {
            return Err(protocol("a message is over the limit"));
        }
        if !fin {
            return Ok(None);
        }
        let data = std::mem::take(&mut self.data);
        match self.opcode.take() {
            Some(TEXT) => String::from_utf8(data)
                .map(|text| Some(Incoming::Text(text)))
                .map_err(|_| protocol("a text message is not UTF-8")),
            _ => Ok(Some(Incoming::Binary(data))),
        }
    }
}

// ---- SHA-1, for the accept key ---------------------------------------------

/// SHA-1 (FIPS 180-4). Here for one use, the `Sec-WebSocket-Accept` check,
/// where it is no security: footplate's tree has SHA-256 (sha2) but no SHA-1
/// in its Cargo.toml, and forty lines are cheaper than a dependency.
pub fn sha1(data: &[u8]) -> [u8; 20] {
    let mut h: [u32; 5] = [0x67452301, 0xEFCDAB89, 0x98BADCFE, 0x10325476, 0xC3D2E1F0];
    let mut msg = data.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&((data.len() as u64) * 8).to_be_bytes());
    for block in msg.chunks(64) {
        let mut w = [0u32; 80];
        for (i, word) in block.chunks(4).enumerate() {
            w[i] = u32::from_be_bytes([word[0], word[1], word[2], word[3]]);
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }
        let [mut a, mut b, mut c, mut d, mut e] = h;
        for (i, word) in w.iter().enumerate() {
            let (f, k) = match i {
                0..=19 => ((b & c) | (!b & d), 0x5A827999),
                20..=39 => (b ^ c ^ d, 0x6ED9EBA1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1BBCDC),
                _ => (b ^ c ^ d, 0xCA62C1D6),
            };
            let next = a
                .rotate_left(5)
                .wrapping_add(f)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(*word);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = next;
        }
        for (slot, add) in h.iter_mut().zip([a, b, c, d, e]) {
            *slot = slot.wrapping_add(add);
        }
    }
    let mut out = [0u8; 20];
    for (i, word) in h.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
    }
    out
}

pub fn accept_key(key: &str) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(sha1(format!("{key}{GUID}").as_bytes()))
}

/// Random bytes from the TLS library's own source: a mask key must not be
/// predictable, and rustls has the generator already.
pub fn random(buf: &mut [u8]) {
    rustls::crypto::ring::default_provider()
        .secure_random
        .fill(buf)
        .expect("the system's random source works");
}

// ---- the connection ---------------------------------------------------------

/// How a server is trusted: by the roots this system trusts, as api.rs's
/// `tls()` has it, except that the ALPN names no protocol: an upgrade is
/// HTTP/1.1.
fn tls() -> io::Result<Arc<rustls::ClientConfig>> {
    static CONFIG: OnceLock<Result<Arc<rustls::ClientConfig>, String>> = OnceLock::new();
    CONFIG
        .get_or_init(|| {
            let mut roots = rustls::RootCertStore::empty();
            let found = rustls_native_certs::load_native_certs();
            roots.add_parsable_certificates(found.certs);
            if roots.is_empty() {
                let why: Vec<String> = found.errors.iter().map(ToString::to_string).collect();
                return Err(format!(
                    "this system's root certificates could not be read: {}",
                    why.join("; ")
                ));
            }
            let provider = Arc::new(rustls::crypto::ring::default_provider());
            let mut config = rustls::ClientConfig::builder_with_provider(provider)
                .with_safe_default_protocol_versions()
                .map_err(|err| err.to_string())?
                .with_root_certificates(roots)
                .with_no_client_auth();
            config.alpn_protocols = vec![b"http/1.1".to_vec()];
            Ok(Arc::new(config))
        })
        .clone()
        .map_err(io::Error::other)
}

enum Stream {
    Plain(TcpStream),
    Tls(Box<rustls::StreamOwned<rustls::ClientConnection, TcpStream>>),
}

impl Stream {
    fn tcp(&self) -> &TcpStream {
        match self {
            Stream::Plain(tcp) => tcp,
            Stream::Tls(tls) => &tls.sock,
        }
    }
}

impl Read for Stream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            Stream::Plain(tcp) => tcp.read(buf),
            Stream::Tls(tls) => tls.read(buf),
        }
    }
}

impl Write for Stream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self {
            Stream::Plain(tcp) => tcp.write(buf),
            Stream::Tls(tls) => tls.write(buf),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            Stream::Plain(tcp) => tcp.flush(),
            Stream::Tls(tls) => tls.flush(),
        }
    }
}

/// Takes what looks like a key out of text a server sent back: OpenAI's 401
/// repeats the key it was given in part, and the harness never prints the key.
pub fn redact(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find("sk-") {
        out.push_str(&rest[..at]);
        out.push_str("sk-…");
        let tail = &rest[at + 3..];
        let end = tail
            .find(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '*')))
            .unwrap_or(tail.len());
        rest = &tail[end..];
    }
    out.push_str(rest);
    out
}

pub struct Conn {
    stream: Stream,
    decoder: Decoder,
    assembler: Assembler,
    /// A message longer than this goes out in fragments of this size.
    pub max_frame: usize,
    close_sent: bool,
}

impl Conn {
    /// Connects, upgrades, and returns with the first frames' bytes (if the
    /// server sent any after its headers) already taken in. `headers` are
    /// sent as given: the caller's, and never printed here.
    pub fn connect(url: &Url, headers: &[(&str, &str)]) -> Result<Conn, Error> {
        let mut last = io::Error::new(
            io::ErrorKind::NotFound,
            format!("{} names no address", url.host),
        );
        let mut tcp = None;
        for addr in (url.host.as_str(), url.port).to_socket_addrs()? {
            match TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT) {
                Ok(conn) => {
                    tcp = Some(conn);
                    break;
                }
                Err(err) => last = err,
            }
        }
        let tcp = tcp.ok_or(last)?;
        // Audio goes in small writes that must not wait for each other.
        tcp.set_nodelay(true)?;
        tcp.set_read_timeout(Some(CONNECT_TIMEOUT))?;
        tcp.set_write_timeout(Some(CONNECT_TIMEOUT))?;
        let stream = if url.tls {
            let name = rustls::pki_types::ServerName::try_from(url.host.clone())
                .map_err(|err| io::Error::new(io::ErrorKind::InvalidInput, err))?;
            let session = rustls::ClientConnection::new(tls()?, name).map_err(io::Error::other)?;
            Stream::Tls(Box::new(rustls::StreamOwned::new(session, tcp)))
        } else {
            Stream::Plain(tcp)
        };
        let mut conn = Conn {
            stream,
            decoder: Decoder::new(false),
            assembler: Assembler::default(),
            max_frame: usize::MAX,
            close_sent: false,
        };
        conn.upgrade(url, headers)?;
        Ok(conn)
    }

    fn upgrade(&mut self, url: &Url, headers: &[(&str, &str)]) -> Result<(), Error> {
        use base64::Engine;
        let mut raw = [0u8; 16];
        random(&mut raw);
        let key = base64::engine::general_purpose::STANDARD.encode(raw);
        let mut request = format!(
            "GET {} HTTP/1.1\r\nHost: {}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\
             Sec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\n",
            url.path,
            url.authority()
        );
        for (name, value) in headers {
            if name.contains(['\r', '\n', ':']) || value.contains(['\r', '\n']) {
                return Err(protocol(format!(
                    "header {name:?} cannot be sent as written"
                )));
            }
            request.push_str(&format!("{name}: {value}\r\n"));
        }
        request.push_str("\r\n");
        self.stream.write_all(request.as_bytes())?;
        self.stream.flush()?;

        let mut head = Vec::new();
        let mut chunk = [0u8; 4096];
        let end = loop {
            if let Some(at) = head.windows(4).position(|w| w == b"\r\n\r\n") {
                break at;
            }
            if head.len() > HEADER_LIMIT {
                return Err(protocol("the server's headers are over 16 KiB"));
            }
            match self.stream.read(&mut chunk)? {
                0 => {
                    return Err(protocol("the server closed before it answered the upgrade"));
                }
                n => head.extend_from_slice(&chunk[..n]),
            }
        };
        let text = String::from_utf8_lossy(&head[..end]).to_string();
        let mut lines = text.split("\r\n");
        let status = lines.next().unwrap_or_default().to_string();
        let header = |wanted: &str| {
            lines
                .clone()
                .filter_map(|line| line.split_once(':'))
                .find(|(name, _)| name.trim().eq_ignore_ascii_case(wanted))
                .map(|(_, value)| value.trim().to_string())
        };
        if status.split_whitespace().nth(1) != Some("101") {
            // Whatever the server said besides, without any key it echoed.
            let mut body = head[end + 4..].to_vec();
            let _ = self.stream.read_to_end_limited(&mut body, 2048);
            let body = redact(&String::from_utf8_lossy(&body));
            return Err(Error::Refused(format!("{status} {}", body.trim())));
        }
        let upgraded = header("upgrade").is_some_and(|v| v.eq_ignore_ascii_case("websocket"));
        let connection = header("connection").is_some_and(|v| {
            v.to_ascii_lowercase()
                .split(',')
                .any(|t| t.trim() == "upgrade")
        });
        if !upgraded || !connection {
            return Err(protocol("the answer is 101 but names no websocket upgrade"));
        }
        if header("sec-websocket-accept").as_deref() != Some(accept_key(&key).as_str()) {
            return Err(protocol(
                "the Sec-WebSocket-Accept key is not the one asked for",
            ));
        }
        if header("sec-websocket-extensions").is_some() {
            return Err(protocol(
                "the server chose an extension that was not offered",
            ));
        }
        // Frames may already follow the headers in what was read.
        self.decoder.push(&head[end + 4..]);
        Ok(())
    }

    /// How long `poll` waits for bytes.
    pub fn set_poll(&mut self, wait: Duration) -> io::Result<()> {
        self.stream.tcp().set_read_timeout(Some(wait))
    }

    fn write_frame(&mut self, fin: bool, opcode: u8, payload: &[u8]) -> Result<(), Error> {
        let mut key = [0u8; 4];
        random(&mut key);
        self.stream
            .write_all(&encode_frame(fin, opcode, payload, Some(key)))?;
        self.stream.flush()?;
        Ok(())
    }

    /// A text message, in fragments where it is longer than `max_frame`.
    pub fn send_text(&mut self, text: &str) -> Result<(), Error> {
        let bytes = text.as_bytes();
        if bytes.len() <= self.max_frame {
            return self.write_frame(true, TEXT, bytes);
        }
        let mut chunks = bytes.chunks(self.max_frame).peekable();
        let mut opcode = TEXT;
        while let Some(chunk) = chunks.next() {
            let fin = chunks.peek().is_none();
            self.write_frame(fin, opcode, chunk)?;
            opcode = CONTINUATION;
        }
        Ok(())
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn send_ping(&mut self, payload: &[u8]) -> Result<(), Error> {
        self.write_frame(true, PING, payload)
    }

    /// The close frame; the connection ends when the far end's comes back.
    pub fn send_close(&mut self, code: u16, reason: &str) -> Result<(), Error> {
        if self.close_sent {
            return Ok(());
        }
        self.close_sent = true;
        let mut body = code.to_be_bytes().to_vec();
        body.extend_from_slice(reason.as_bytes());
        self.write_frame(true, CLOSE, &body)
    }

    /// What arrives within the poll interval, as messages. A ping is answered
    /// here with its own bytes before it is returned (so a caller may count
    /// it), and a close from the far end is answered with a close.
    pub fn poll(&mut self) -> Result<Vec<Incoming>, Error> {
        let mut chunk = [0u8; 16 << 10];
        match self.stream.read(&mut chunk) {
            Ok(0) => return Err(Error::Closed),
            Ok(n) => self.decoder.push(&chunk[..n]),
            Err(err)
                if matches!(
                    err.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) => {}
            Err(err) => return Err(err.into()),
        }
        let mut out = Vec::new();
        while let Some(frame) = self.decoder.next_frame()? {
            if let Some(message) = self.assembler.accept(frame)? {
                match &message {
                    Incoming::Ping(body) => self.write_frame(true, PONG, body)?,
                    Incoming::Close(code, _) if !self.close_sent => {
                        self.send_close(code.unwrap_or(1000), "")?;
                    }
                    _ => {}
                }
                out.push(message);
            }
        }
        Ok(out)
    }
}

/// `read_to_end` that stops at a size and at a timeout: a refusal's body is
/// small, and the server may keep the connection open after it.
trait ReadLimited {
    fn read_to_end_limited(&mut self, out: &mut Vec<u8>, limit: usize) -> io::Result<()>;
}

impl ReadLimited for Stream {
    fn read_to_end_limited(&mut self, out: &mut Vec<u8>, limit: usize) -> io::Result<()> {
        let _ = self
            .tcp()
            .set_read_timeout(Some(Duration::from_millis(500)));
        let mut chunk = [0u8; 1024];
        while out.len() < limit {
            match self.read(&mut chunk) {
                Ok(0) => break,
                Ok(n) => out.extend_from_slice(&chunk[..n]),
                Err(_) => break,
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    fn decode_all(wire: &[u8], expect_masked: bool) -> Vec<Frame> {
        let mut decoder = Decoder::new(expect_masked);
        decoder.push(wire);
        let mut out = Vec::new();
        while let Some(frame) = decoder.next_frame().unwrap() {
            out.push(frame);
        }
        out
    }

    #[test]
    fn sha1_and_the_accept_key_are_the_rfcs() {
        assert_eq!(
            hex(&sha1(b"abc")),
            "a9993e364706816aba3e25717850c26c9cd0d89d"
        );
        // A million bytes: many blocks, the length in the padding.
        assert_eq!(
            hex(&sha1(&vec![b'a'; 1_000_000])),
            "34aa973cd4c4daa4f61eeb2bdbad27316534016f"
        );
        assert_eq!(
            hex(&sha1(
                b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"
            )),
            "84983e441c3bd26ebaae4aa1f95129e5e54670f1"
        );
        // RFC 6455 section 1.3.
        assert_eq!(
            accept_key("dGhlIHNhbXBsZSBub25jZQ=="),
            "s3pPLMBiTxaQ9kYGzzhZRbK+xOo="
        );
    }

    #[test]
    fn frames_are_the_rfcs_bytes() {
        // RFC 6455 section 5.7: "Hello" unmasked, and masked with 0x37fa213d.
        assert_eq!(
            encode_frame(true, TEXT, b"Hello", None),
            [0x81, 0x05, 0x48, 0x65, 0x6c, 0x6c, 0x6f]
        );
        let masked = [
            0x81, 0x85, 0x37, 0xfa, 0x21, 0x3d, 0x7f, 0x9f, 0x4d, 0x51, 0x58,
        ];
        assert_eq!(
            encode_frame(true, TEXT, b"Hello", Some([0x37, 0xfa, 0x21, 0x3d])),
            masked
        );
        // The masked ping response is the same bytes with opcode 10.
        assert_eq!(
            encode_frame(true, PONG, b"Hello", Some([0x37, 0xfa, 0x21, 0x3d]))[2..],
            masked[2..]
        );
        // Two fragments of "Hello".
        assert_eq!(
            encode_frame(false, TEXT, b"Hel", None),
            [0x01, 0x03, 0x48, 0x65, 0x6c]
        );
        assert_eq!(
            encode_frame(true, CONTINUATION, b"lo", None),
            [0x80, 0x02, 0x6c, 0x6f]
        );
        // 256 bytes use a 16-bit length, 64 KiB a 64-bit one.
        let bytes = vec![7u8; 256];
        let frame = encode_frame(true, BINARY, &bytes, None);
        assert_eq!(frame[..4], [0x82, 0x7e, 0x01, 0x00]);
        assert_eq!(frame.len(), 4 + 256);
        let bytes = vec![7u8; 65536];
        let frame = encode_frame(true, BINARY, &bytes, None);
        assert_eq!(frame[..10], [0x82, 0x7f, 0, 0, 0, 0, 0, 1, 0, 0]);
        assert_eq!(frame.len(), 10 + 65536);
        // And what a client writes, a server reads back unmasked.
        let wire = encode_frame(true, BINARY, &bytes, Some([1, 2, 3, 4]));
        let frames = decode_all(&wire, true);
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].payload, bytes);
    }

    #[test]
    fn a_decoder_waits_for_the_rest_however_the_bytes_are_cut() {
        let mut wire = encode_frame(true, TEXT, b"Hello", None);
        wire.extend(encode_frame(true, BINARY, &vec![9u8; 300], None));
        let mut decoder = Decoder::new(false);
        let mut got = Vec::new();
        for byte in &wire {
            decoder.push(&[*byte]);
            while let Some(frame) = decoder.next_frame().unwrap() {
                got.push(frame);
            }
        }
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].payload, b"Hello");
        assert_eq!(got[1].payload.len(), 300);
    }

    #[test]
    fn a_decoder_refuses_what_the_rfc_has_a_client_close_on() {
        let refuses = |wire: &[u8]| {
            let mut decoder = Decoder::new(false);
            decoder.push(wire);
            decoder.next_frame().is_err()
        };
        // A masked frame from the server.
        assert!(refuses(&[0x81, 0x85, 1, 2, 3, 4, 0, 0, 0, 0, 0]));
        // A reserved bit, a reserved opcode.
        assert!(refuses(&[0xC1, 0x00]));
        assert!(refuses(&[0x83, 0x00]));
        // A ping of 126 bytes, a fragmented ping.
        assert!(refuses(&[0x89, 0x7e, 0x00, 0x7e]));
        assert!(refuses(&[0x09, 0x00]));
        // A length with the top bit set.
        assert!(refuses(&[0x82, 0x7f, 0x80, 0, 0, 0, 0, 0, 0, 0]));
        // And the stand-in's side: a client frame that is not masked.
        let mut server = Decoder::new(true);
        server.push(&[0x81, 0x00]);
        assert!(server.next_frame().is_err());
    }

    #[test]
    fn fragments_join_with_a_ping_between_them() {
        let mut asm = Assembler::default();
        let mut wire = encode_frame(false, TEXT, b"Hel", None);
        wire.extend(encode_frame(true, PING, b"mid", None));
        wire.extend(encode_frame(true, CONTINUATION, b"lo", None));
        let got: Vec<_> = decode_all(&wire, false)
            .into_iter()
            .filter_map(|frame| asm.accept(frame).unwrap())
            .collect();
        assert_eq!(
            got,
            [
                Incoming::Ping(b"mid".to_vec()),
                Incoming::Text("Hello".into())
            ]
        );
        // The orders the RFC forbids.
        let mut asm = Assembler::default();
        assert!(
            asm.accept(Frame {
                fin: true,
                opcode: CONTINUATION,
                payload: vec![]
            })
            .is_err()
        );
        asm.accept(Frame {
            fin: false,
            opcode: TEXT,
            payload: b"a".to_vec(),
        })
        .unwrap();
        assert!(
            asm.accept(Frame {
                fin: true,
                opcode: TEXT,
                payload: b"b".to_vec()
            })
            .is_err()
        );
        let mut asm = Assembler::default();
        assert!(
            asm.accept(Frame {
                fin: true,
                opcode: TEXT,
                payload: vec![0xff, 0xfe]
            })
            .is_err()
        );
        // A close, with its code and reason.
        let mut body = 1001u16.to_be_bytes().to_vec();
        body.extend_from_slice(b"going away");
        assert_eq!(
            Assembler::default()
                .accept(Frame {
                    fin: true,
                    opcode: CLOSE,
                    payload: body
                })
                .unwrap(),
            Some(Incoming::Close(Some(1001), "going away".into()))
        );
        // The cap is checked before extending a partial message.
        let mut data = b"abc".to_vec();
        assert!(extend_bounded(&mut data, b"d", 3).is_err());
        assert_eq!(data, b"abc");
        extend_bounded(&mut data, b"", 3).unwrap();
        assert_eq!(data, b"abc");
    }

    #[test]
    fn addresses_parse() {
        let url = Url::parse("wss://api.openai.com/v1/live/sessions").unwrap();
        assert_eq!(
            (url.tls, url.host.as_str(), url.port),
            (true, "api.openai.com", 443)
        );
        assert_eq!(url.path, "/v1/live/sessions");
        assert_eq!(url.authority(), "api.openai.com");
        let url = Url::parse("ws://127.0.0.1:8123").unwrap();
        assert_eq!((url.tls, url.port, url.path.as_str()), (false, 8123, "/"));
        assert_eq!(url.authority(), "127.0.0.1:8123");
        assert!(Url::parse("https://example.com").is_err());
        assert!(Url::parse("ws://:80/").is_err());
    }

    #[test]
    fn a_key_in_a_refusal_is_taken_out() {
        assert_eq!(
            redact("Incorrect API key provided: sk-proj-ab***yz. You can find it"),
            "Incorrect API key provided: sk-…. You can find it"
        );
        assert_eq!(redact("no key here"), "no key here");
    }
}
