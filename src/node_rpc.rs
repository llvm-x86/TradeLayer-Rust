//! Minimal HTTP/1.1 JSON-RPC client for a trusted Litecoin or Bitcoin Core
//! block source, built on `std::net` only.
//!
//! Scope: `getblockcount`, `getblockhash` and `getblock(hash, 0)`. Credentials
//! come from environment variables and are never printed, logged or copied
//! into error values. The endpoint is loopback-only unless the caller passes
//! `EndpointPolicy::ExplicitRemoteOptIn`. Responses are bounded before they
//! allocate. Nothing here validates the node's answers beyond transport and
//! JSON-RPC envelope checks. Block validity is decided by `node_follower`.

use crate::chain::BlockId;
use crate::node_follower::{BlockSource, SourceError};
use crate::raw_block_ingest::MAX_UNVERIFIED_RAW_BLOCK_BYTES;
use std::io::{ErrorKind, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

pub const RPC_USER_ENV: &str = "TRADELAYER_RPC_USER";
pub const RPC_PASSWORD_ENV: &str = "TRADELAYER_RPC_PASSWORD";
/// One whole HTTP response body: a 4 MB raw block as hex, plus its envelope.
pub const MAX_RPC_RESPONSE_BYTES: usize = 8 * 1024 * 1024;
const MAX_HEADER_BYTES: usize = 16 * 1024;
const MAX_JSON_DEPTH: usize = 8;
pub const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
pub const DEFAULT_IO_TIMEOUT: Duration = Duration::from_secs(30);

/// Basic-auth credentials. Debug output is redacted, and the type has no
/// accessor that returns the password or the user name.
#[derive(Clone)]
pub struct RpcCredentials {
    basic: String,
}

impl RpcCredentials {
    pub fn new(user: &str, password: &str) -> Result<Self, SourceError> {
        let user_ok = !user.is_empty() && user.bytes().all(|b| b > 0x20 && b != b':' && b != 0x7f);
        let password_ok = !password.is_empty() && password.bytes().all(|b| b >= 0x20 && b != 0x7f);
        if !user_ok || !password_ok {
            return Err(SourceError::Config("RPC credential format"));
        }
        let joined = format!("{user}:{password}");
        Ok(Self {
            basic: format!("Basic {}", base64_encode(joined.as_bytes())),
        })
    }

    /// Reads [`RPC_USER_ENV`] and [`RPC_PASSWORD_ENV`].
    pub fn from_env() -> Result<Self, SourceError> {
        Self::from_env_vars(RPC_USER_ENV, RPC_PASSWORD_ENV)
    }

    pub fn from_env_vars(
        user_var: &'static str,
        password_var: &'static str,
    ) -> Result<Self, SourceError> {
        let user = std::env::var(user_var).map_err(|_| SourceError::MissingCredential(user_var))?;
        let password = std::env::var(password_var)
            .map_err(|_| SourceError::MissingCredential(password_var))?;
        Self::new(&user, &password)
    }

    fn authorization(&self) -> &str {
        &self.basic
    }
}

impl core::fmt::Debug for RpcCredentials {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "RpcCredentials(<redacted>)")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndpointPolicy {
    LoopbackOnly,
    /// Credentials then cross the network in cleartext. Use only with a
    /// tunnel or a network you control.
    ExplicitRemoteOptIn,
}

#[derive(Debug, Clone)]
pub struct RpcConfig {
    addr: SocketAddr,
    credentials: RpcCredentials,
    connect_timeout: Duration,
    io_timeout: Duration,
}

impl RpcConfig {
    pub fn new(
        addr: SocketAddr,
        credentials: RpcCredentials,
        policy: EndpointPolicy,
    ) -> Result<Self, SourceError> {
        if policy == EndpointPolicy::LoopbackOnly && !is_loopback(addr) {
            return Err(SourceError::NonLoopbackRefused);
        }
        Ok(Self {
            addr,
            credentials,
            connect_timeout: DEFAULT_CONNECT_TIMEOUT,
            io_timeout: DEFAULT_IO_TIMEOUT,
        })
    }

    pub fn with_timeouts(mut self, connect: Duration, io: Duration) -> Result<Self, SourceError> {
        if connect.is_zero() || io.is_zero() {
            return Err(SourceError::Config("zero timeout"));
        }
        self.connect_timeout = connect;
        self.io_timeout = io;
        Ok(self)
    }
}

fn is_loopback(addr: SocketAddr) -> bool {
    let ip = addr.ip();
    ip.is_loopback()
        || match ip {
            std::net::IpAddr::V6(v6) => v6.to_ipv4_mapped().is_some_and(|v4| v4.is_loopback()),
            std::net::IpAddr::V4(_) => false,
        }
}

/// A [`BlockSource`] backed by a Core-compatible JSON-RPC endpoint. Each call
/// opens one connection and sends `Connection: close`.
#[derive(Debug, Clone)]
pub struct RpcBlockSource {
    config: RpcConfig,
}

impl RpcBlockSource {
    pub fn new(config: RpcConfig) -> Self {
        Self { config }
    }

    fn call(&self, method: &'static str, params: &str) -> Result<Json, SourceError> {
        let body =
            format!("{{\"jsonrpc\":\"1.0\",\"id\":1,\"method\":\"{method}\",\"params\":{params}}}");
        let mut stream = TcpStream::connect_timeout(&self.config.addr, self.config.connect_timeout)
            .map_err(io_error)?;
        stream
            .set_read_timeout(Some(self.config.io_timeout))
            .map_err(io_error)?;
        stream
            .set_write_timeout(Some(self.config.io_timeout))
            .map_err(io_error)?;
        let head = format!(
            "POST / HTTP/1.1\r\nHost: {addr}\r\nAuthorization: {auth}\r\n\
             Content-Type: application/json\r\nContent-Length: {len}\r\nConnection: close\r\n\r\n",
            addr = self.config.addr,
            auth = self.config.credentials.authorization(),
            len = body.len(),
        );
        stream.write_all(head.as_bytes()).map_err(io_error)?;
        stream.write_all(body.as_bytes()).map_err(io_error)?;
        let (status, payload) = read_http_response(&mut stream)?;
        match status {
            200 | 500 => parse_rpc_envelope(&payload),
            401 => Err(SourceError::Unauthorized),
            other => Err(SourceError::HttpStatus(other)),
        }
    }
}

impl BlockSource for RpcBlockSource {
    fn best_height(&self) -> Result<u64, SourceError> {
        let value = self.call("getblockcount", "[]")?;
        as_u64(&value).ok_or(SourceError::UnexpectedResult("block count"))
    }

    fn block_hash_at(&self, height: u64) -> Result<BlockId, SourceError> {
        let value = self.call("getblockhash", &format!("[{height}]"))?;
        match value {
            Json::Str(text) => hash_from_hex(&text),
            _ => Err(SourceError::UnexpectedResult("block hash")),
        }
    }

    fn raw_block(&self, hash: BlockId) -> Result<Vec<u8>, SourceError> {
        let params = format!("[\"{}\",0]", encode_hex(&hash.0));
        let value = self.call("getblock", &params)?;
        match value {
            Json::Str(text) => decode_hex(&text, MAX_UNVERIFIED_RAW_BLOCK_BYTES),
            _ => Err(SourceError::UnexpectedResult("raw block")),
        }
    }
}

fn io_error(err: std::io::Error) -> SourceError {
    match err.kind() {
        ErrorKind::TimedOut | ErrorKind::WouldBlock => SourceError::Timeout,
        kind => SourceError::Io(kind),
    }
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

/// Reads one HTTP/1.x response. Requires `Content-Length`, rejects chunked
/// encoding, and never allocates more than `MAX_RPC_RESPONSE_BYTES`.
fn read_http_response(stream: &mut impl Read) -> Result<(u16, Vec<u8>), SourceError> {
    let mut pending = Vec::new();
    let mut chunk = [0u8; 4096];
    let header_end = loop {
        if let Some(position) = find_subslice(&pending, b"\r\n\r\n") {
            break position;
        }
        if pending.len() > MAX_HEADER_BYTES {
            return Err(SourceError::MalformedHttp("headers too large"));
        }
        let read = stream.read(&mut chunk).map_err(io_error)?;
        if read == 0 {
            return Err(SourceError::ShortResponse);
        }
        pending.extend_from_slice(&chunk[..read]);
    };
    if header_end > MAX_HEADER_BYTES {
        return Err(SourceError::MalformedHttp("headers too large"));
    }
    let head = std::str::from_utf8(&pending[..header_end])
        .map_err(|_| SourceError::MalformedHttp("non-UTF-8 headers"))?;
    let mut lines = head.split("\r\n");
    let status_line = lines
        .next()
        .ok_or(SourceError::MalformedHttp("empty response"))?;
    let mut parts = status_line.splitn(3, ' ');
    if !matches!(parts.next(), Some("HTTP/1.1" | "HTTP/1.0")) {
        return Err(SourceError::MalformedHttp("status line"));
    }
    let status = parts
        .next()
        .filter(|code| code.len() == 3 && code.bytes().all(|b| b.is_ascii_digit()))
        .and_then(|code| code.parse::<u16>().ok())
        .ok_or(SourceError::MalformedHttp("status code"))?;
    let mut content_length: Option<usize> = None;
    for line in lines {
        let (name, value) = line
            .split_once(':')
            .ok_or(SourceError::MalformedHttp("header line"))?;
        let name = name.to_ascii_lowercase();
        if name == "transfer-encoding" {
            return Err(SourceError::MalformedHttp("chunked encoding unsupported"));
        }
        if name == "content-length" {
            if content_length.is_some() {
                return Err(SourceError::MalformedHttp("duplicate content-length"));
            }
            let value = value.trim();
            if value.is_empty() || value.len() > 12 || !value.bytes().all(|b| b.is_ascii_digit()) {
                return Err(SourceError::MalformedHttp("content-length"));
            }
            content_length = Some(
                value
                    .parse::<usize>()
                    .map_err(|_| SourceError::MalformedHttp("content-length"))?,
            );
        }
    }
    let length = content_length.ok_or(SourceError::MalformedHttp("missing content-length"))?;
    if length > MAX_RPC_RESPONSE_BYTES {
        return Err(SourceError::ResponseTooLarge);
    }
    let leftover = &pending[header_end + 4..];
    let mut body = Vec::with_capacity(length);
    body.extend_from_slice(&leftover[..leftover.len().min(length)]);
    while body.len() < length {
        let want = (length - body.len()).min(chunk.len());
        let read = stream.read(&mut chunk[..want]).map_err(io_error)?;
        if read == 0 {
            return Err(SourceError::ShortResponse);
        }
        body.extend_from_slice(&chunk[..read]);
    }
    Ok((status, body))
}

/// Checks the JSON-RPC 1.0/2.0 envelope and returns `result`. A non-null
/// `error` yields its numeric code only. The node's message text is dropped.
fn parse_rpc_envelope(payload: &[u8]) -> Result<Json, SourceError> {
    let Json::Object(fields) = parse_json(payload)? else {
        return Err(SourceError::MalformedJson("not an object"));
    };
    if let Some(error) = field(&fields, "error") {
        if !matches!(error, Json::Null) {
            let Json::Object(error_fields) = error else {
                return Err(SourceError::MalformedJson("error is not an object"));
            };
            let code = field(error_fields, "code")
                .and_then(as_i64)
                .ok_or(SourceError::MalformedJson("error code"))?;
            return Err(SourceError::RpcError(code));
        }
    }
    match field(&fields, "id").and_then(as_u64) {
        Some(1) => {}
        _ => return Err(SourceError::RpcIdMismatch),
    }
    field(&fields, "result")
        .cloned()
        .ok_or(SourceError::MalformedJson("missing result"))
}

fn field<'a>(fields: &'a [(String, Json)], name: &str) -> Option<&'a Json> {
    fields
        .iter()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value)
}

fn as_u64(value: &Json) -> Option<u64> {
    match value {
        Json::Number(text) if !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit()) => {
            text.parse().ok()
        }
        _ => None,
    }
}

fn as_i64(value: &Json) -> Option<i64> {
    match value {
        Json::Number(text) => text.parse().ok(),
        _ => None,
    }
}

fn hash_from_hex(text: &str) -> Result<BlockId, SourceError> {
    if text.len() != 64 {
        return Err(SourceError::UnexpectedResult("block hash length"));
    }
    let bytes = decode_hex(text, 32)?;
    let array: [u8; 32] = bytes
        .try_into()
        .map_err(|_| SourceError::UnexpectedResult("block hash length"))?;
    Ok(BlockId(array))
}

fn decode_hex(text: &str, max_bytes: usize) -> Result<Vec<u8>, SourceError> {
    let bytes = text.as_bytes();
    if !bytes.len().is_multiple_of(2) {
        return Err(SourceError::MalformedHex);
    }
    if bytes.len() / 2 > max_bytes {
        return Err(SourceError::BlockTooLarge);
    }
    bytes
        .chunks_exact(2)
        .map(|pair| -> Result<u8, SourceError> { Ok((nibble(pair[0])? << 4) | nibble(pair[1])?) })
        .collect()
}

fn nibble(value: u8) -> Result<u8, SourceError> {
    match value {
        b'0'..=b'9' => Ok(value - b'0'),
        b'a'..=b'f' => Ok(value - b'a' + 10),
        b'A'..=b'F' => Ok(value - b'A' + 10),
        _ => Err(SourceError::MalformedHex),
    }
}

fn encode_hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &byte in bytes {
        out.push(DIGITS[usize::from(byte >> 4)] as char);
        out.push(DIGITS[usize::from(byte & 0x0f)] as char);
    }
    out
}

fn base64_encode(input: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b0 = u32::from(chunk[0]);
        let b1 = u32::from(chunk.get(1).copied().unwrap_or(0));
        let b2 = u32::from(chunk.get(2).copied().unwrap_or(0));
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(TABLE[(n >> 18) as usize & 63] as char);
        out.push(TABLE[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            TABLE[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            TABLE[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

/// Minimal JSON value. Numbers keep their source text, so integer parsing is
/// explicit and no float rounding can occur.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Json {
    Null,
    Bool(bool),
    Number(String),
    Str(String),
    Array(Vec<Json>),
    Object(Vec<(String, Json)>),
}

fn parse_json(input: &[u8]) -> Result<Json, SourceError> {
    let mut parser = JsonParser { input, pos: 0 };
    let value = parser.value(0)?;
    parser.skip_ws();
    if parser.pos != input.len() {
        return Err(SourceError::MalformedJson("trailing data"));
    }
    Ok(value)
}

struct JsonParser<'a> {
    input: &'a [u8],
    pos: usize,
}

impl JsonParser<'_> {
    fn skip_ws(&mut self) {
        while matches!(self.input.get(self.pos), Some(b' ' | b'\t' | b'\r' | b'\n')) {
            self.pos += 1;
        }
    }

    fn peek(&self) -> Option<u8> {
        self.input.get(self.pos).copied()
    }

    fn expect(&mut self, byte: u8) -> Result<(), SourceError> {
        if self.peek() == Some(byte) {
            self.pos += 1;
            Ok(())
        } else {
            Err(SourceError::MalformedJson("unexpected token"))
        }
    }

    fn literal(&mut self, text: &[u8], value: Json) -> Result<Json, SourceError> {
        if self.input[self.pos..].starts_with(text) {
            self.pos += text.len();
            Ok(value)
        } else {
            Err(SourceError::MalformedJson("invalid literal"))
        }
    }

    fn value(&mut self, depth: usize) -> Result<Json, SourceError> {
        if depth > MAX_JSON_DEPTH {
            return Err(SourceError::MalformedJson("nesting too deep"));
        }
        self.skip_ws();
        match self.peek() {
            Some(b'{') => self.object(depth),
            Some(b'[') => self.array(depth),
            Some(b'"') => self.string().map(Json::Str),
            Some(b't') => self.literal(b"true", Json::Bool(true)),
            Some(b'f') => self.literal(b"false", Json::Bool(false)),
            Some(b'n') => self.literal(b"null", Json::Null),
            Some(b'-' | b'0'..=b'9') => self.number(),
            _ => Err(SourceError::MalformedJson("unexpected token")),
        }
    }

    fn object(&mut self, depth: usize) -> Result<Json, SourceError> {
        self.expect(b'{')?;
        let mut fields: Vec<(String, Json)> = Vec::new();
        self.skip_ws();
        if self.peek() == Some(b'}') {
            self.pos += 1;
            return Ok(Json::Object(fields));
        }
        loop {
            self.skip_ws();
            let key = self.string()?;
            if fields.iter().any(|(existing, _)| *existing == key) {
                return Err(SourceError::MalformedJson("duplicate key"));
            }
            self.skip_ws();
            self.expect(b':')?;
            let value = self.value(depth + 1)?;
            fields.push((key, value));
            self.skip_ws();
            match self.peek() {
                Some(b',') => self.pos += 1,
                Some(b'}') => {
                    self.pos += 1;
                    return Ok(Json::Object(fields));
                }
                _ => return Err(SourceError::MalformedJson("object syntax")),
            }
        }
    }

    fn array(&mut self, depth: usize) -> Result<Json, SourceError> {
        self.expect(b'[')?;
        let mut items = Vec::new();
        self.skip_ws();
        if self.peek() == Some(b']') {
            self.pos += 1;
            return Ok(Json::Array(items));
        }
        loop {
            items.push(self.value(depth + 1)?);
            self.skip_ws();
            match self.peek() {
                Some(b',') => self.pos += 1,
                Some(b']') => {
                    self.pos += 1;
                    return Ok(Json::Array(items));
                }
                _ => return Err(SourceError::MalformedJson("array syntax")),
            }
        }
    }

    fn string(&mut self) -> Result<String, SourceError> {
        self.expect(b'"')?;
        let mut out = String::new();
        loop {
            let byte = self
                .peek()
                .ok_or(SourceError::MalformedJson("unterminated string"))?;
            self.pos += 1;
            match byte {
                b'"' => return Ok(out),
                b'\\' => {
                    let escape = self
                        .peek()
                        .ok_or(SourceError::MalformedJson("unterminated escape"))?;
                    self.pos += 1;
                    let decoded = match escape {
                        b'"' => '"',
                        b'\\' => '\\',
                        b'/' => '/',
                        b'b' => '\u{8}',
                        b'f' => '\u{c}',
                        b'n' => '\n',
                        b'r' => '\r',
                        b't' => '\t',
                        b'u' => {
                            let hex = self
                                .input
                                .get(self.pos..self.pos + 4)
                                .ok_or(SourceError::MalformedJson("short unicode escape"))?;
                            self.pos += 4;
                            let mut code = 0u32;
                            for &digit in hex {
                                code = (code << 4) | u32::from(nibble(digit)?);
                            }
                            char::from_u32(code)
                                .ok_or(SourceError::MalformedJson("unicode escape"))?
                        }
                        _ => return Err(SourceError::MalformedJson("invalid escape")),
                    };
                    out.push(decoded);
                }
                0x00..=0x1f => return Err(SourceError::MalformedJson("control character")),
                _ => {
                    // Copy one UTF-8 scalar; the input is validated byte-wise
                    // as ASCII or continued UTF-8 below.
                    let start = self.pos - 1;
                    let width = utf8_width(byte).ok_or(SourceError::MalformedJson("utf-8"))?;
                    let end = start + width;
                    let slice = self
                        .input
                        .get(start..end)
                        .ok_or(SourceError::MalformedJson("utf-8"))?;
                    let text = std::str::from_utf8(slice)
                        .map_err(|_| SourceError::MalformedJson("utf-8"))?;
                    out.push_str(text);
                    self.pos = end;
                }
            }
        }
    }

    fn number(&mut self) -> Result<Json, SourceError> {
        let start = self.pos;
        if self.peek() == Some(b'-') {
            self.pos += 1;
        }
        let digits_start = self.pos;
        while matches!(self.peek(), Some(b'0'..=b'9')) {
            self.pos += 1;
        }
        if self.pos == digits_start {
            return Err(SourceError::MalformedJson("number"));
        }
        if self.peek() == Some(b'.') {
            self.pos += 1;
            let fraction = self.pos;
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.pos += 1;
            }
            if self.pos == fraction {
                return Err(SourceError::MalformedJson("number"));
            }
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            self.pos += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.pos += 1;
            }
            let exponent = self.pos;
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.pos += 1;
            }
            if self.pos == exponent {
                return Err(SourceError::MalformedJson("number"));
            }
        }
        let text = std::str::from_utf8(&self.input[start..self.pos])
            .map_err(|_| SourceError::MalformedJson("number"))?;
        Ok(Json::Number(text.to_owned()))
    }
}

fn utf8_width(lead: u8) -> Option<usize> {
    match lead {
        0x00..=0x7f => Some(1),
        0xc2..=0xdf => Some(2),
        0xe0..=0xef => Some(3),
        0xf0..=0xf4 => Some(4),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_rfc4648_vectors() {
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
        assert_eq!(base64_encode(b"foob"), "Zm9vYg==");
        assert_eq!(base64_encode(b"user:pass"), "dXNlcjpwYXNz");
    }

    #[test]
    fn json_accepts_envelope_and_rejects_duplicates_and_trailing_data() {
        let ok = parse_json(br#"{"result":"ab","error":null,"id":1}"#).unwrap();
        assert!(matches!(ok, Json::Object(_)));
        assert!(parse_json(br#"{"result":1,"result":2,"id":1}"#).is_err());
        assert!(parse_json(br#"{"result":1,"id":1} x"#).is_err());
        assert!(parse_json(br#"{"a":1.}"#).is_err());
    }

    #[test]
    fn credentials_reject_colon_in_user_and_redact_debug() {
        assert!(RpcCredentials::new("a:b", "pw").is_err());
        assert!(RpcCredentials::new("", "pw").is_err());
        let creds = RpcCredentials::new("tl", "hunter2").unwrap();
        assert!(!format!("{creds:?}").contains("hunter2"));
    }

    #[test]
    fn remote_endpoint_requires_explicit_policy() {
        let creds = RpcCredentials::new("tl", "pw").unwrap();
        let remote: SocketAddr = "10.1.2.3:8332".parse().unwrap();
        assert_eq!(
            RpcConfig::new(remote, creds.clone(), EndpointPolicy::LoopbackOnly).unwrap_err(),
            SourceError::NonLoopbackRefused
        );
        assert!(RpcConfig::new(remote, creds, EndpointPolicy::ExplicitRemoteOptIn).is_ok());
    }
}
