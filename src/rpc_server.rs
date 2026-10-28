//! Read-only JSON-RPC 2.0 over HTTP/1.1 on `std::net`, serving one restored
//! TLJP checkpoint. Method shapes are DRAFT until diffed against the JS/C++
//! node; this module makes no parity claim. No method writes state.
//!
//! Transport: one request per connection, `POST /` only, `Content-Length`
//! bodies only (chunked is refused). Batch arrays and notifications (requests
//! without `id`) are rejected with -32600. Params must be a positional array.

use std::fmt;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{IpAddr, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use crate::joint::JointProtocolState;
use crate::ledger::{AccountId, Property, PropertyId, PropertyKind};
use crate::network_profile::TradeLayerMainnet;

pub const MAX_REQUEST_BODY_BYTES: usize = 64 * 1024;
pub const MAX_HEADER_BYTES: usize = 8 * 1024;
pub const MAX_CONCURRENT_CONNECTIONS: usize = 16;
pub const CONNECTION_TIMEOUT: Duration = Duration::from_secs(5);
pub const MAX_JSON_DEPTH: usize = 8;
pub const MAX_JSON_VALUES: usize = 512;
pub const MAX_JSON_STRING_BYTES: usize = 1024;
pub const MAX_JSON_NUMBER_BYTES: usize = 32;

/// Refuses a non-loopback bind unless the caller explicitly allows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BindError {
    NonLoopback(SocketAddr),
}

impl fmt::Display for BindError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonLoopback(addr) => write!(
                f,
                "refusing non-loopback bind {addr}; pass --allow-nonloopback to override"
            ),
        }
    }
}

impl std::error::Error for BindError {}

pub fn check_bind_policy(addr: SocketAddr, allow_nonloopback: bool) -> Result<(), BindError> {
    let loopback = match addr.ip() {
        IpAddr::V4(ip) => ip.is_loopback(),
        IpAddr::V6(ip) => ip.is_loopback(),
    };
    if loopback || allow_nonloopback {
        Ok(())
    } else {
        Err(BindError::NonLoopback(addr))
    }
}

/// Transport-level failures. Each maps to one HTTP status and no JSON body.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HttpError {
    BadRequest,
    NotFound,
    MethodNotAllowed,
    RequestTimeout,
    LengthRequired,
    PayloadTooLarge,
    HeaderTooLarge,
    NotImplemented,
}

impl HttpError {
    pub const fn code(self) -> u16 {
        match self {
            Self::BadRequest => 400,
            Self::NotFound => 404,
            Self::MethodNotAllowed => 405,
            Self::RequestTimeout => 408,
            Self::LengthRequired => 411,
            Self::PayloadTooLarge => 413,
            Self::HeaderTooLarge => 431,
            Self::NotImplemented => 501,
        }
    }
}

// ---------------------------------------------------------------------------
// Bounded JSON parser. Recursion depth is capped at MAX_JSON_DEPTH, the total
// number of values at MAX_JSON_VALUES, and strings and numbers at fixed sizes.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
enum Json {
    Null,
    Boolean,
    Number(String),
    String(String),
    Array(Vec<Json>),
    Object(Vec<(String, Json)>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ParseError {
    Syntax,
    Limit,
    DuplicateKey,
}

struct Parser<'a> {
    bytes: &'a [u8],
    pos: usize,
    values: usize,
}

fn parse_json(bytes: &[u8]) -> Result<Json, ParseError> {
    let mut parser = Parser {
        bytes,
        pos: 0,
        values: 0,
    };
    let value = parser.value(0)?;
    parser.skip_ws();
    if parser.pos != bytes.len() {
        return Err(ParseError::Syntax);
    }
    Ok(value)
}

impl Parser<'_> {
    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn bump(&mut self) -> Result<u8, ParseError> {
        let byte = self.peek().ok_or(ParseError::Syntax)?;
        self.pos += 1;
        Ok(byte)
    }

    fn skip_ws(&mut self) {
        while let Some(b' ' | b'\t' | b'\n' | b'\r') = self.peek() {
            self.pos += 1;
        }
    }

    fn expect(&mut self, byte: u8) -> Result<(), ParseError> {
        if self.bump()? == byte {
            Ok(())
        } else {
            Err(ParseError::Syntax)
        }
    }

    fn literal(&mut self, text: &[u8]) -> Result<(), ParseError> {
        for &byte in text {
            self.expect(byte)?;
        }
        Ok(())
    }

    fn value(&mut self, depth: usize) -> Result<Json, ParseError> {
        self.skip_ws();
        self.values += 1;
        if self.values > MAX_JSON_VALUES {
            return Err(ParseError::Limit);
        }
        match self.peek().ok_or(ParseError::Syntax)? {
            b'{' => self.object(depth + 1),
            b'[' => self.array(depth + 1),
            b'"' => self.string().map(Json::String),
            b't' => self.literal(b"true").map(|()| Json::Boolean),
            b'f' => self.literal(b"false").map(|()| Json::Boolean),
            b'n' => self.literal(b"null").map(|()| Json::Null),
            b'-' | b'0'..=b'9' => self.number(),
            _ => Err(ParseError::Syntax),
        }
    }

    fn array(&mut self, depth: usize) -> Result<Json, ParseError> {
        if depth > MAX_JSON_DEPTH {
            return Err(ParseError::Limit);
        }
        self.expect(b'[')?;
        let mut items = Vec::new();
        self.skip_ws();
        if self.peek() == Some(b']') {
            self.pos += 1;
            return Ok(Json::Array(items));
        }
        loop {
            items.push(self.value(depth)?);
            self.skip_ws();
            match self.bump()? {
                b',' => {}
                b']' => return Ok(Json::Array(items)),
                _ => return Err(ParseError::Syntax),
            }
        }
    }

    fn object(&mut self, depth: usize) -> Result<Json, ParseError> {
        if depth > MAX_JSON_DEPTH {
            return Err(ParseError::Limit);
        }
        self.expect(b'{')?;
        let mut members: Vec<(String, Json)> = Vec::new();
        self.skip_ws();
        if self.peek() == Some(b'}') {
            self.pos += 1;
            return Ok(Json::Object(members));
        }
        loop {
            self.skip_ws();
            if self.peek() != Some(b'"') {
                return Err(ParseError::Syntax);
            }
            let key = self.string()?;
            self.skip_ws();
            self.expect(b':')?;
            let value = self.value(depth)?;
            if members.iter().any(|(existing, _)| *existing == key) {
                return Err(ParseError::DuplicateKey);
            }
            members.push((key, value));
            self.skip_ws();
            match self.bump()? {
                b',' => {}
                b'}' => return Ok(Json::Object(members)),
                _ => return Err(ParseError::Syntax),
            }
        }
    }

    fn string(&mut self) -> Result<String, ParseError> {
        self.expect(b'"')?;
        let mut out = Vec::new();
        loop {
            let byte = self.bump()?;
            match byte {
                b'"' => break,
                0x00..=0x1f => return Err(ParseError::Syntax),
                b'\\' => match self.bump()? {
                    b'"' => out.push(b'"'),
                    b'\\' => out.push(b'\\'),
                    b'/' => out.push(b'/'),
                    b'b' => out.push(0x08),
                    b'f' => out.push(0x0c),
                    b'n' => out.push(b'\n'),
                    b'r' => out.push(b'\r'),
                    b't' => out.push(b'\t'),
                    b'u' => {
                        let scalar = self.unicode_escape()?;
                        let mut buf = [0_u8; 4];
                        out.extend_from_slice(scalar.encode_utf8(&mut buf).as_bytes());
                    }
                    _ => return Err(ParseError::Syntax),
                },
                _ => out.push(byte),
            }
            if out.len() > MAX_JSON_STRING_BYTES {
                return Err(ParseError::Limit);
            }
        }
        String::from_utf8(out).map_err(|_| ParseError::Syntax)
    }

    fn unicode_escape(&mut self) -> Result<char, ParseError> {
        let high = self.hex4()?;
        let code = if (0xD800..=0xDBFF).contains(&high) {
            self.expect(b'\\')?;
            self.expect(b'u')?;
            let low = self.hex4()?;
            if !(0xDC00..=0xDFFF).contains(&low) {
                return Err(ParseError::Syntax);
            }
            0x10000 + ((high - 0xD800) << 10) + (low - 0xDC00)
        } else if (0xDC00..=0xDFFF).contains(&high) {
            return Err(ParseError::Syntax);
        } else {
            high
        };
        char::from_u32(code).ok_or(ParseError::Syntax)
    }

    fn hex4(&mut self) -> Result<u32, ParseError> {
        let mut value = 0_u32;
        for _ in 0..4 {
            let digit = match self.bump()? {
                byte @ b'0'..=b'9' => byte - b'0',
                byte @ b'a'..=b'f' => byte - b'a' + 10,
                byte @ b'A'..=b'F' => byte - b'A' + 10,
                _ => return Err(ParseError::Syntax),
            };
            value = value * 16 + u32::from(digit);
        }
        Ok(value)
    }

    fn digits(&mut self) -> bool {
        let start = self.pos;
        while let Some(b'0'..=b'9') = self.peek() {
            self.pos += 1;
        }
        self.pos > start
    }

    fn number(&mut self) -> Result<Json, ParseError> {
        let start = self.pos;
        if self.peek() == Some(b'-') {
            self.pos += 1;
        }
        match self.peek() {
            Some(b'0') => self.pos += 1,
            Some(b'1'..=b'9') => {
                self.digits();
            }
            _ => return Err(ParseError::Syntax),
        }
        if self.peek() == Some(b'.') {
            self.pos += 1;
            if !self.digits() {
                return Err(ParseError::Syntax);
            }
        }
        if let Some(b'e' | b'E') = self.peek() {
            self.pos += 1;
            if let Some(b'+' | b'-') = self.peek() {
                self.pos += 1;
            }
            if !self.digits() {
                return Err(ParseError::Syntax);
            }
        }
        let token = &self.bytes[start..self.pos];
        if token.len() > MAX_JSON_NUMBER_BYTES {
            return Err(ParseError::Limit);
        }
        std::str::from_utf8(token)
            .map(|text| Json::Number(text.to_owned()))
            .map_err(|_| ParseError::Syntax)
    }
}

fn lookup<'a>(members: &'a [(String, Json)], key: &str) -> Option<&'a Json> {
    members
        .iter()
        .find(|(name, _)| name.as_str() == key)
        .map(|(_, value)| value)
}

// ---------------------------------------------------------------------------
// JSON-RPC layer. Output is built by hand so key order and number formatting
// are fixed. Money is always a JSON string of integer atoms.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RpcError {
    code: i64,
    message: &'static str,
}

const PARSE_ERROR: RpcError = RpcError {
    code: -32700,
    message: "Parse error",
};
const INVALID_REQUEST: RpcError = RpcError {
    code: -32600,
    message: "Invalid Request",
};
const METHOD_NOT_FOUND: RpcError = RpcError {
    code: -32601,
    message: "Method not found",
};
const INVALID_PARAMS: RpcError = RpcError {
    code: -32602,
    message: "Invalid params",
};
/// Server-defined (outside the reserved -32768..-32000 JSON-RPC ranges).
const PROPERTY_NOT_FOUND: RpcError = RpcError {
    code: -32001,
    message: "Property not found",
};

enum Id {
    Null,
    Text(String),
    /// Raw integer token from the request, echoed verbatim.
    Integer(String),
}

impl Id {
    fn render(&self) -> String {
        match self {
            Self::Null => "null".to_owned(),
            Self::Text(text) => json_string(text),
            Self::Integer(token) => token.clone(),
        }
    }
}

/// Pure request handler: request bytes in, JSON response bytes out. No
/// sockets, no state mutation.
pub fn handle(state: &JointProtocolState, request: &[u8]) -> Vec<u8> {
    if request.len() > MAX_REQUEST_BODY_BYTES {
        return error_response(&Id::Null, INVALID_REQUEST);
    }
    let value = match parse_json(request) {
        Ok(value) => value,
        Err(ParseError::Syntax) => return error_response(&Id::Null, PARSE_ERROR),
        Err(ParseError::Limit | ParseError::DuplicateKey) => {
            return error_response(&Id::Null, INVALID_REQUEST)
        }
    };
    let Json::Object(members) = &value else {
        // Batch arrays and scalar roots are rejected.
        return error_response(&Id::Null, INVALID_REQUEST);
    };
    let id = match lookup(members, "id") {
        Some(Json::Null) => Id::Null,
        Some(Json::String(text)) => Id::Text(text.clone()),
        Some(Json::Number(token)) if is_integer_token(token) => Id::Integer(token.clone()),
        _ => return error_response(&Id::Null, INVALID_REQUEST),
    };
    match lookup(members, "jsonrpc") {
        Some(Json::String(version)) if version.as_str() == "2.0" => {}
        _ => return error_response(&id, INVALID_REQUEST),
    }
    let method = match lookup(members, "method") {
        Some(Json::String(name)) => name.as_str(),
        _ => return error_response(&id, INVALID_REQUEST),
    };
    let params: &[Json] = match lookup(members, "params") {
        None => &[],
        Some(Json::Array(items)) => items,
        Some(_) => return error_response(&id, INVALID_PARAMS),
    };
    match dispatch(state, method, params) {
        Ok(result) => ok_response(&id, &result),
        Err(err) => error_response(&id, err),
    }
}

fn dispatch(state: &JointProtocolState, method: &str, params: &[Json]) -> Result<String, RpcError> {
    match method {
        "tl_getinfo" => {
            expect_arity(params, 0)?;
            Ok(format!(
                "{{\"network\":{},\"finalized_height\":{},\"property_count\":{},\"processed_count\":{}}}",
                json_string(network_name(state.network())),
                height_json(state.finalized_height()),
                state.ledger().property_count(),
                state.processed_count()
            ))
        }
        "tl_getproperty" => {
            expect_arity(params, 1)?;
            let id = property_param(&params[0])?;
            let property = require_property(state, id)?;
            Ok(property_json(id, property))
        }
        "tl_listproperties" => {
            expect_arity(params, 0)?;
            let items: Vec<String> = state
                .ledger()
                .properties()
                .map(|(id, property)| property_json(id, property))
                .collect();
            Ok(format!("[{}]", items.join(",")))
        }
        "tl_getbalance" => {
            expect_arity(params, 2)?;
            let address = account_param(&params[0])?;
            let id = property_param(&params[1])?;
            require_property(state, id)?;
            let balance = state.ledger().available_atoms(id, &address);
            Ok(format!(
                "{{\"address\":{},\"propertyid\":{},\"balance\":\"{}\"}}",
                json_string(address.as_str()),
                id.get(),
                balance
            ))
        }
        "tl_getallbalancesforaddress" => {
            expect_arity(params, 1)?;
            let address = account_param(&params[0])?;
            let items: Vec<String> = state
                .ledger()
                .available_balances_of(&address)
                .iter()
                .map(|(id, atoms)| {
                    format!("{{\"propertyid\":{},\"balance\":\"{}\"}}", id.get(), atoms)
                })
                .collect();
            Ok(format!("[{}]", items.join(",")))
        }
        "tl_getsyncstatus" => {
            expect_arity(params, 0)?;
            Ok(format!(
                "{{\"source\":\"checkpoint\",\"finalized_height\":{},\"processed_count\":{}}}",
                height_json(state.finalized_height()),
                state.processed_count()
            ))
        }
        _ => Err(METHOD_NOT_FOUND),
    }
}

fn expect_arity(params: &[Json], expected: usize) -> Result<(), RpcError> {
    if params.len() == expected {
        Ok(())
    } else {
        Err(INVALID_PARAMS)
    }
}

fn property_param(value: &Json) -> Result<PropertyId, RpcError> {
    let Json::Number(token) = value else {
        return Err(INVALID_PARAMS);
    };
    if !is_canonical_unsigned(token) {
        return Err(INVALID_PARAMS);
    }
    let number: u32 = token.parse().map_err(|_| INVALID_PARAMS)?;
    PropertyId::try_from(number).map_err(|_| INVALID_PARAMS)
}

fn account_param(value: &Json) -> Result<AccountId, RpcError> {
    let Json::String(text) = value else {
        return Err(INVALID_PARAMS);
    };
    AccountId::try_from(text.as_str()).map_err(|_| INVALID_PARAMS)
}

fn require_property(state: &JointProtocolState, id: PropertyId) -> Result<&Property, RpcError> {
    state.ledger().property(id).ok_or(PROPERTY_NOT_FOUND)
}

fn property_json(id: PropertyId, property: &Property) -> String {
    format!(
        "{{\"propertyid\":{},\"issuer\":{},\"kind\":{},\"total_supply_atoms\":\"{}\"}}",
        id.get(),
        json_string(property.issuer.as_str()),
        json_string(kind_name(property.kind)),
        property.total_supply_atoms
    )
}

fn kind_name(kind: PropertyKind) -> &'static str {
    match kind {
        PropertyKind::Fixed => "fixed",
        PropertyKind::Managed => "managed",
    }
}

fn network_name(network: TradeLayerMainnet) -> &'static str {
    match network {
        TradeLayerMainnet::Bitcoin => "bitcoin",
        TradeLayerMainnet::Litecoin => "litecoin",
        TradeLayerMainnet::Dogecoin => "dogecoin",
    }
}

fn height_json(height: Option<u64>) -> String {
    match height {
        Some(value) => value.to_string(),
        None => "null".to_owned(),
    }
}

fn is_canonical_unsigned(token: &str) -> bool {
    !token.is_empty()
        && token.bytes().all(|byte| byte.is_ascii_digit())
        && (token == "0" || !token.starts_with('0'))
}

fn is_integer_token(token: &str) -> bool {
    let digits = token.strip_prefix('-').unwrap_or(token);
    is_canonical_unsigned(digits)
}

fn json_string(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for ch in text.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn ok_response(id: &Id, result: &str) -> Vec<u8> {
    format!(
        "{{\"jsonrpc\":\"2.0\",\"result\":{result},\"id\":{}}}",
        id.render()
    )
    .into_bytes()
}

fn error_response(id: &Id, error: RpcError) -> Vec<u8> {
    format!(
        "{{\"jsonrpc\":\"2.0\",\"error\":{{\"code\":{},\"message\":{}}},\"id\":{}}}",
        error.code,
        json_string(error.message),
        id.render()
    )
    .into_bytes()
}

// ---------------------------------------------------------------------------
// HTTP/1.1 framing. The body is never read or allocated until the declared
// Content-Length has passed the cap.
// ---------------------------------------------------------------------------

fn is_token_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&byte)
}

fn io_to_http(err: &io::Error) -> HttpError {
    match err.kind() {
        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock => HttpError::RequestTimeout,
        _ => HttpError::BadRequest,
    }
}

/// Reads one head byte-at-a-time so the reader is left exactly at the body.
fn read_head<R: BufRead>(reader: &mut R) -> Result<Vec<u8>, HttpError> {
    let mut head = Vec::new();
    let mut byte = [0_u8; 1];
    loop {
        match reader.read(&mut byte) {
            Ok(0) => return Err(HttpError::BadRequest),
            Ok(_) => head.push(byte[0]),
            Err(err) if err.kind() == io::ErrorKind::Interrupted => continue,
            Err(err) => return Err(io_to_http(&err)),
        }
        if head.len() > MAX_HEADER_BYTES {
            return Err(HttpError::HeaderTooLarge);
        }
        if head.ends_with(b"\r\n\r\n") {
            return Ok(head);
        }
    }
}

fn parse_head(head: &[u8]) -> Result<usize, HttpError> {
    let text = std::str::from_utf8(head).map_err(|_| HttpError::BadRequest)?;
    let text = text.strip_suffix("\r\n\r\n").ok_or(HttpError::BadRequest)?;
    let mut lines = text.split("\r\n");
    let request_line = lines.next().ok_or(HttpError::BadRequest)?;
    let mut parts = request_line.split(' ');
    let (method, target, version) = (parts.next(), parts.next(), parts.next());
    if parts.next().is_some() {
        return Err(HttpError::BadRequest);
    }
    let (Some(method), Some(target), Some(version)) = (method, target, version) else {
        return Err(HttpError::BadRequest);
    };
    if version != "HTTP/1.1" && version != "HTTP/1.0" {
        return Err(HttpError::BadRequest);
    }
    if method != "POST" {
        return Err(HttpError::MethodNotAllowed);
    }
    if target != "/" {
        return Err(HttpError::NotFound);
    }

    let mut content_length: Option<u64> = None;
    for line in lines {
        if line.starts_with(' ') || line.starts_with('\t') {
            return Err(HttpError::BadRequest);
        }
        if line.bytes().any(|byte| byte == b'\r' || byte == b'\n') {
            return Err(HttpError::BadRequest);
        }
        let (name, value) = line.split_once(':').ok_or(HttpError::BadRequest)?;
        if name.is_empty() || !name.bytes().all(is_token_byte) {
            return Err(HttpError::BadRequest);
        }
        let value = value.trim_matches(|c| c == ' ' || c == '\t');
        if name.eq_ignore_ascii_case("transfer-encoding") {
            return Err(HttpError::NotImplemented);
        }
        if name.eq_ignore_ascii_case("content-length") {
            if content_length.is_some() {
                return Err(HttpError::BadRequest);
            }
            if value.is_empty() || value.len() > 20 || !value.bytes().all(|b| b.is_ascii_digit()) {
                return Err(HttpError::BadRequest);
            }
            content_length = Some(value.parse::<u64>().map_err(|_| HttpError::BadRequest)?);
        }
    }
    let length = content_length.ok_or(HttpError::LengthRequired)?;
    if length > MAX_REQUEST_BODY_BYTES as u64 {
        return Err(HttpError::PayloadTooLarge);
    }
    usize::try_from(length).map_err(|_| HttpError::PayloadTooLarge)
}

/// Reads one request and returns its body. Declared length is validated
/// before any body byte is read or any buffer is sized from it.
pub fn read_http_body<R: BufRead>(reader: &mut R) -> Result<Vec<u8>, HttpError> {
    let head = read_head(reader)?;
    let length = parse_head(&head)?;
    let mut body = Vec::with_capacity(length);
    let mut limited = reader.by_ref().take(length as u64);
    limited
        .read_to_end(&mut body)
        .map_err(|err| io_to_http(&err))?;
    if body.len() != length {
        return Err(HttpError::BadRequest);
    }
    Ok(body)
}

fn reason_phrase(status: u16) -> &'static str {
    match status {
        200 => "OK",
        400 => "Bad Request",
        404 => "Not Found",
        405 => "Method Not Allowed",
        408 => "Request Timeout",
        411 => "Length Required",
        413 => "Payload Too Large",
        431 => "Request Header Fields Too Large",
        501 => "Not Implemented",
        503 => "Service Unavailable",
        _ => "Error",
    }
}

fn write_response(stream: &TcpStream, status: u16, body: &[u8]) -> io::Result<()> {
    stream.set_write_timeout(Some(CONNECTION_TIMEOUT))?;
    let content_type = if body.is_empty() {
        ""
    } else {
        "Content-Type: application/json\r\n"
    };
    let head = format!(
        "HTTP/1.1 {status} {}\r\nContent-Length: {}\r\n{content_type}Connection: close\r\n\r\n",
        reason_phrase(status),
        body.len()
    );
    let mut out: &TcpStream = stream;
    out.write_all(head.as_bytes())?;
    out.write_all(body)?;
    out.flush()
}

/// Read side with an overall deadline for the whole request, so a client
/// trickling bytes cannot extend the 5 s budget per read.
struct DeadlineReader<'a> {
    stream: &'a TcpStream,
    deadline: Instant,
}

impl Read for DeadlineReader<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let remaining = self
            .deadline
            .checked_duration_since(Instant::now())
            .filter(|left| !left.is_zero())
            .ok_or_else(|| io::Error::from(io::ErrorKind::TimedOut))?;
        self.stream.set_read_timeout(Some(remaining))?;
        let mut stream: &TcpStream = self.stream;
        stream.read(buf)
    }
}

/// Serves exactly one request on an accepted connection, then closes it.
pub fn serve_connection(stream: &TcpStream, state: &JointProtocolState) -> io::Result<()> {
    let deadline = Instant::now() + CONNECTION_TIMEOUT;
    let mut reader = BufReader::new(DeadlineReader { stream, deadline });
    match read_http_body(&mut reader) {
        Ok(body) => write_response(stream, 200, &handle(state, &body)),
        Err(err) => write_response(stream, err.code(), &[]),
    }
}

struct ActiveSlot(Arc<AtomicUsize>);

impl Drop for ActiveSlot {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

/// Accept loop: at most MAX_CONCURRENT_CONNECTIONS handler threads. Excess
/// connections receive 503 immediately and are closed.
pub fn serve(listener: TcpListener, state: Arc<JointProtocolState>) -> io::Result<()> {
    let active = Arc::new(AtomicUsize::new(0));
    for incoming in listener.incoming() {
        let stream = match incoming {
            Ok(stream) => stream,
            Err(_) => {
                thread::sleep(Duration::from_millis(10));
                continue;
            }
        };
        if active.fetch_add(1, Ordering::AcqRel) >= MAX_CONCURRENT_CONNECTIONS {
            active.fetch_sub(1, Ordering::AcqRel);
            let _ = write_response(&stream, 503, &[]);
            continue;
        }
        let slot = ActiveSlot(Arc::clone(&active));
        let state = Arc::clone(&state);
        let _ = thread::Builder::new().spawn(move || {
            let _slot = slot;
            let _ = serve_connection(&stream, &state);
        });
    }
    Ok(())
}
