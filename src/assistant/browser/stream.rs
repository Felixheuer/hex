//! agent-browser's live stream of one browser (`stream enable`): a
//! WebSocket on localhost that sends the viewport as JPEG frames, the page's
//! address and tabs, and takes the person's mouse and keys back. Agent-Desk shows
//! it as a task's live preview and lets the person drive it after Take over
//! (`rune_ui::assistant_overlay::live`).
//!
//! The socket is a plain RFC 6455 client over `TcpStream`: localhost only,
//! text frames, no extensions, so no WebSocket crate is needed.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::Path;
use std::time::Duration;

use base64::Engine as _;
use serde_json::{Value, json};

/// The largest message read; a 4K frame at full quality is far below it.
const MAX_MESSAGE: u64 = 32 * 1024 * 1024;

/// Turns on `session`'s stream and returns its port.
pub fn enable(path: &Path, session: &str) -> Result<u16, String> {
    if let Ok(status) = super::call(path, session, &["stream", "status"])
        && let Some(port) = port(&status)
    {
        return Ok(port);
    }
    let enabled = super::call(path, session, &["stream", "enable"])?;
    if let Some(port) = port(&enabled) {
        return Ok(port);
    }
    let status = super::call(path, session, &["stream", "status"])?;
    port(&status).ok_or_else(|| "agent-browser named no stream port".into())
}

/// Turns `session`'s stream off again.
pub fn disable(path: &Path, session: &str) {
    let _ = super::call(path, session, &["stream", "disable"]);
}

/// The port in a `stream status`/`stream enable` reply, wherever it sits.
pub fn port(data: &Value) -> Option<u16> {
    match data {
        Value::Object(map) => map
            .get("port")
            .and_then(Value::as_u64)
            .and_then(|port| u16::try_from(port).ok())
            .filter(|port| *port > 0)
            .or_else(|| map.values().find_map(port)),
        _ => None,
    }
}

/// What the stream said.
#[derive(Clone, Debug, PartialEq)]
pub enum Message {
    /// A viewport image (JPEG, as the stream asks for, but sniff it) and
    /// the size of the page it shows, in CSS pixels.
    Frame {
        seq: u64,
        jpeg: Vec<u8>,
        viewport: (f32, f32),
    },
    /// The active tab's address changed.
    Url(String),
    /// The tabs, and which is active: its title and address.
    Tabs { title: String, url: String },
    /// Anything else (status, console).
    Other,
}

/// One text message from the stream; `None` when it isn't one.
pub fn parse(text: &str) -> Option<Message> {
    let value: Value = serde_json::from_str(text).ok()?;
    Some(match value.get("type")?.as_str()? {
        "frame" => {
            let data = value.get("data")?.as_str()?;
            let jpeg = base64::engine::general_purpose::STANDARD
                .decode(data)
                .ok()?;
            let metadata = value.get("metadata");
            let number = |name| {
                metadata
                    .and_then(|metadata| metadata.get(name))
                    .and_then(Value::as_f64)
                    .unwrap_or(0.0) as f32
            };
            Message::Frame {
                seq: value.get("seq").and_then(Value::as_u64).unwrap_or(0),
                jpeg,
                viewport: (number("deviceWidth"), number("deviceHeight")),
            }
        }
        "url" => Message::Url(text_field(&value, "url")?),
        "tabs" => {
            let tabs = value.get("tabs").and_then(Value::as_array)?;
            let active = tabs
                .iter()
                .find(|tab| tab.get("active").and_then(Value::as_bool) == Some(true))
                .or_else(|| tabs.first())?;
            Message::Tabs {
                title: text_field(active, "title").unwrap_or_default(),
                url: text_field(active, "url").unwrap_or_default(),
            }
        }
        _ => Message::Other,
    })
}

fn text_field(value: &Value, name: &str) -> Option<String> {
    value.get(name).and_then(Value::as_str).map(str::to_owned)
}

/// The acknowledgement that lets the next frame come (ack pacing).
pub fn ack(seq: u64) -> String {
    json!({"type": "ack", "seq": seq}).to_string()
}

/// At most `fps` frames a second from now on.
pub fn max_fps(fps: u32) -> String {
    json!({"type": "config", "maxFps": fps}).to_string()
}

/// A mouse button.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Button {
    None,
    Left,
    Right,
}

impl Button {
    fn name(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Left => "left",
            Self::Right => "right",
        }
    }
}

/// What the person does on the preview, in the page's CSS pixels.
#[derive(Clone, Debug, PartialEq)]
pub enum Input {
    Move {
        x: f32,
        y: f32,
    },
    Press {
        x: f32,
        y: f32,
        button: Button,
        clicks: u32,
    },
    Release {
        x: f32,
        y: f32,
        button: Button,
        clicks: u32,
    },
    Wheel {
        x: f32,
        y: f32,
        dx: f32,
        dy: f32,
    },
    /// A key with its DOM name (`Enter`, `a`) and the text it types, if any.
    Key {
        key: String,
        text: Option<String>,
    },
}

/// The stream messages that replay `input` in the browser.
pub fn encode(input: &Input) -> Vec<String> {
    let mouse = |kind: &str, x: f32, y: f32, button: Button, clicks: u32| {
        json!({
            "type": "input_mouse",
            "eventType": kind,
            "x": x,
            "y": y,
            "button": button.name(),
            "clickCount": clicks,
        })
        .to_string()
    };
    match input {
        Input::Move { x, y } => vec![mouse("mouseMoved", *x, *y, Button::None, 0)],
        Input::Press {
            x,
            y,
            button,
            clicks,
        } => vec![mouse("mousePressed", *x, *y, *button, *clicks)],
        Input::Release {
            x,
            y,
            button,
            clicks,
        } => vec![mouse("mouseReleased", *x, *y, *button, *clicks)],
        Input::Wheel { x, y, dx, dy } => vec![
            json!({
                "type": "input_mouse",
                "eventType": "mouseWheel",
                "x": x,
                "y": y,
                "deltaX": dx,
                "deltaY": dy,
            })
            .to_string(),
        ],
        Input::Key { key, text } => {
            let mut down = json!({"type": "input_keyboard", "eventType": "keyDown", "key": key});
            if let Some(text) = text {
                down["text"] = Value::String(text.clone());
            }
            let up = json!({"type": "input_keyboard", "eventType": "keyUp", "key": key});
            vec![down.to_string(), up.to_string()]
        }
    }
}

/// The reading half of a stream's socket.
pub struct Reader {
    stream: BufReader<TcpStream>,
    /// Answers pings from inside [`Reader::next`].
    pong: Writer,
}

/// The writing half; clone it into whatever sends input.
#[derive(Clone)]
pub struct Writer {
    stream: std::sync::Arc<std::sync::Mutex<TcpStream>>,
}

/// Connects to the stream on `port`, `query` appended to its address
/// (`pacing=ack&maxFps=3`).
pub fn connect(port: u16, query: &str) -> std::io::Result<(Reader, Writer)> {
    let stream = TcpStream::connect(("127.0.0.1", port))?;
    stream.set_nodelay(true)?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;
    let mut key = [0u8; 16];
    getrandom::fill(&mut key).map_err(|error| std::io::Error::other(error.to_string()))?;
    let key = base64::engine::general_purpose::STANDARD.encode(key);
    let mut writer = stream.try_clone()?;
    write!(
        writer,
        "GET /?{query} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nUpgrade: websocket\r\n\
         Connection: Upgrade\r\nSec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\n\
         Origin: http://127.0.0.1\r\n\r\n"
    )?;
    writer.flush()?;
    let mut reader = BufReader::new(stream);
    let mut status = String::new();
    reader.read_line(&mut status)?;
    if !status
        .split_whitespace()
        .nth(1)
        .is_some_and(|code| code == "101")
    {
        return Err(std::io::Error::other(format!(
            "the stream refused the connection: {}",
            status.trim()
        )));
    }
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 || line.trim().is_empty() {
            break;
        }
    }
    let writer = Writer {
        stream: std::sync::Arc::new(std::sync::Mutex::new(writer)),
    };
    Ok((
        Reader {
            stream: reader,
            pong: writer.clone(),
        },
        writer,
    ))
}

impl Reader {
    /// The next text message; `None` once the stream closed.
    pub fn next(&mut self) -> std::io::Result<Option<String>> {
        let mut message: Vec<u8> = Vec::new();
        loop {
            let mut head = [0u8; 2];
            if let Err(error) = self.stream.read_exact(&mut head) {
                return match error.kind() {
                    std::io::ErrorKind::UnexpectedEof => Ok(None),
                    _ => Err(error),
                };
            }
            let fin = head[0] & 0x80 != 0;
            let opcode = head[0] & 0x0f;
            let masked = head[1] & 0x80 != 0;
            let length = match head[1] & 0x7f {
                126 => {
                    let mut bytes = [0u8; 2];
                    self.stream.read_exact(&mut bytes)?;
                    u64::from(u16::from_be_bytes(bytes))
                }
                127 => {
                    let mut bytes = [0u8; 8];
                    self.stream.read_exact(&mut bytes)?;
                    u64::from_be_bytes(bytes)
                }
                short => u64::from(short),
            };
            if length > MAX_MESSAGE || message.len() as u64 + length > MAX_MESSAGE {
                return Err(std::io::Error::other("a stream message was too large"));
            }
            let mut mask = [0u8; 4];
            if masked {
                self.stream.read_exact(&mut mask)?;
            }
            let mut payload = vec![0u8; length as usize];
            self.stream.read_exact(&mut payload)?;
            if masked {
                for (index, byte) in payload.iter_mut().enumerate() {
                    *byte ^= mask[index % 4];
                }
            }
            match opcode {
                // Continuation, text, binary.
                0..=2 => {
                    message.extend_from_slice(&payload);
                    if fin {
                        return Ok(Some(String::from_utf8_lossy(&message).into_owned()));
                    }
                }
                8 => return Ok(None),
                9 => self.pong.frame(0xA, &payload)?,
                _ => {}
            }
        }
    }
}

impl Writer {
    /// Sends one text message.
    pub fn send(&self, text: &str) -> std::io::Result<()> {
        self.frame(0x1, text.as_bytes())
    }

    /// Closes the socket; the reader then ends.
    pub fn close(&self) {
        let _ = self.frame(0x8, &[]);
        if let Ok(stream) = self.stream.lock() {
            let _ = stream.shutdown(std::net::Shutdown::Both);
        }
    }

    fn frame(&self, opcode: u8, payload: &[u8]) -> std::io::Result<()> {
        let mut mask = [0u8; 4];
        getrandom::fill(&mut mask).map_err(|error| std::io::Error::other(error.to_string()))?;
        let bytes = frame(opcode, payload, mask);
        let mut stream = self
            .stream
            .lock()
            .map_err(|_| std::io::Error::other("the stream's writer is poisoned"))?;
        stream.write_all(&bytes)?;
        stream.flush()
    }
}

/// A final client frame: masked, as RFC 6455 requires of clients.
fn frame(opcode: u8, payload: &[u8], mask: [u8; 4]) -> Vec<u8> {
    let mut bytes = vec![0x80 | opcode];
    match payload.len() {
        len @ 0..=125 => bytes.push(0x80 | len as u8),
        len @ 126..=0xffff => {
            bytes.push(0x80 | 126);
            bytes.extend_from_slice(&(len as u16).to_be_bytes());
        }
        len => {
            bytes.push(0x80 | 127);
            bytes.extend_from_slice(&(len as u64).to_be_bytes());
        }
    }
    bytes.extend_from_slice(&mask);
    bytes.extend(
        payload
            .iter()
            .enumerate()
            .map(|(index, byte)| byte ^ mask[index % 4]),
    );
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_port_is_found_wherever_the_reply_puts_it() {
        assert_eq!(port(&json!({"enabled": true, "port": 51234})), Some(51234));
        assert_eq!(port(&json!({"stream": {"port": 9}})), Some(9));
        assert_eq!(port(&json!({"enabled": false, "port": 0})), None);
        assert_eq!(port(&json!({"enabled": false})), None);
    }

    #[test]
    fn frames_addresses_and_tabs_are_read() {
        let jpeg = [0xFF, 0xD8, 0xFF, 0xD9];
        let data = base64::engine::general_purpose::STANDARD.encode(jpeg);
        let frame = format!(
            r#"{{"type":"frame","seq":41,"data":"{data}","metadata":{{"deviceWidth":1280,"deviceHeight":720}}}}"#
        );
        assert_eq!(
            parse(&frame),
            Some(Message::Frame {
                seq: 41,
                jpeg: jpeg.to_vec(),
                viewport: (1280.0, 720.0),
            })
        );
        assert_eq!(
            parse(r#"{"type":"url","url":"https://example.com/"}"#),
            Some(Message::Url("https://example.com/".into()))
        );
        assert_eq!(
            parse(
                r#"{"type":"tabs","tabs":[{"title":"A","url":"a"},{"title":"B","url":"b","active":true}]}"#
            ),
            Some(Message::Tabs {
                title: "B".into(),
                url: "b".into()
            })
        );
        assert_eq!(parse(r#"{"type":"status"}"#), Some(Message::Other));
        assert_eq!(parse("not json"), None);
        assert_eq!(parse(r#"{"type":"frame","data":"%%%"}"#), None);
    }

    #[test]
    fn input_becomes_the_streams_messages() {
        let press = encode(&Input::Press {
            x: 40.0,
            y: 20.0,
            button: Button::Left,
            clicks: 1,
        });
        let value: Value = serde_json::from_str(&press[0]).unwrap();
        assert_eq!(value["type"], "input_mouse");
        assert_eq!(value["eventType"], "mousePressed");
        assert_eq!(value["button"], "left");
        assert_eq!(value["clickCount"], 1);
        let key = encode(&Input::Key {
            key: "a".into(),
            text: Some("a".into()),
        });
        assert_eq!(key.len(), 2);
        let down: Value = serde_json::from_str(&key[0]).unwrap();
        assert_eq!(
            (down["eventType"].as_str(), down["text"].as_str()),
            (Some("keyDown"), Some("a"))
        );
        let wheel: Value = serde_json::from_str(
            &encode(&Input::Wheel {
                x: 1.0,
                y: 2.0,
                dx: 0.0,
                dy: 120.0,
            })[0],
        )
        .unwrap();
        assert_eq!(wheel["eventType"], "mouseWheel");
        assert_eq!(wheel["deltaY"], 120.0);
        assert_eq!(ack(7), r#"{"seq":7,"type":"ack"}"#);
    }

    #[test]
    fn client_frames_are_masked_with_their_length() {
        let bytes = frame(0x1, b"hi", [1, 2, 3, 4]);
        assert_eq!(bytes[..2], [0x81, 0x82]);
        assert_eq!(bytes[2..6], [1, 2, 3, 4]);
        assert_eq!(bytes[6..], [b'h' ^ 1, b'i' ^ 2]);
        let long = frame(0x1, &[0; 300], [0; 4]);
        assert_eq!(long[1], 0x80 | 126);
        assert_eq!(u16::from_be_bytes([long[2], long[3]]), 300);
    }

    /// A tiny server: answers the upgrade, sends a fragmented text message
    /// and a ping, reads the pong, then closes.
    #[test]
    fn the_client_reads_messages_and_answers_pings() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut request = String::new();
            while reader.read_line(&mut request).unwrap() > 2 {
                if request.ends_with("\r\n\r\n") {
                    break;
                }
            }
            assert!(request.starts_with("GET /?pacing=ack HTTP/1.1"));
            let mut stream = stream;
            stream
                .write_all(b"HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\n\r\n")
                .unwrap();
            // "hel" + "lo" in two fragments, then a ping.
            stream.write_all(&[0x01, 3, b'h', b'e', b'l']).unwrap();
            stream.write_all(&[0x80, 2, b'l', b'o']).unwrap();
            stream.write_all(&[0x89, 1, b'p']).unwrap();
            let mut pong = [0u8; 7];
            reader.read_exact(&mut pong).unwrap();
            assert_eq!(pong[0], 0x8A);
            assert_eq!(pong[6] ^ pong[2], b'p');
            stream.write_all(&[0x88, 0]).unwrap();
        });
        let (mut reader, _writer) = connect(port, "pacing=ack").unwrap();
        assert_eq!(reader.next().unwrap().as_deref(), Some("hello"));
        assert_eq!(reader.next().unwrap(), None);
        server.join().unwrap();
    }
}
