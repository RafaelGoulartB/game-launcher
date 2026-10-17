//! Talks to a running Steam client through its CEF remote debugging interface
//! (port of Heroic's `nonesteamgame/steamClient.ts`; the same mechanism as
//! Valve's steamos-devkit and Decky Loader). It is the only way to manage
//! non-Steam shortcuts while Steam runs: Steam reads `shortcuts.vdf` at start
//! and rewrites it on exit, so editing the file meanwhile is lost.
//!
//! Steam only opens the interface (HTTP + WebSocket on `127.0.0.1`) when the
//! flag file [`FLAG_FILE`] exists in its install folder at start. Older
//! clients listen on port 8080; current ones pick a port at each start, which
//! [`debugger_addr`] finds.
//!
//! The HTTP and WebSocket clients are minimal on purpose: plain TCP to a fixed
//! local address, no TLS, one request per connection.

use std::hash::{BuildHasher, Hasher};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::path::Path;
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use heronite_i18n::tr;
use serde_json::{json, Value};

/// Created in the Steam folder so the next Steam start exposes the interface.
pub const FLAG_FILE: &str = ".cef-enable-remote-debugging";
/// Where older Steam clients serve the debugger.
pub const DEFAULT_ADDR: &str = "127.0.0.1:8080";

/// Address of the running Steam's debugger: the port its `steamwebhelper`
/// was started with (`--remote-debugging-port=<n>`), else [`DEFAULT_ADDR`].
pub fn debugger_addr() -> String {
    debugger_port(Path::new("/proc")).map(|p| format!("127.0.0.1:{p}")).unwrap_or_else(|| DEFAULT_ADDR.to_string())
}

/// The debugging port in the command line of a `steamwebhelper` under `proc`.
fn debugger_port(proc: &Path) -> Option<u16> {
    std::fs::read_dir(proc).ok()?.flatten().find_map(|entry| {
        let name = entry.file_name();
        if !name.to_string_lossy().bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        let cmdline = std::fs::read(entry.path().join("cmdline")).ok()?;
        let mut args = cmdline.split(|b| *b == 0).map(String::from_utf8_lossy);
        if !args.next()?.rsplit('/').next()?.starts_with("steamwebhelper") {
            return None;
        }
        args.find_map(|a| a.strip_prefix("--remote-debugging-port=").and_then(|p| p.parse::<u16>().ok()).filter(|p| *p != 0))
    })
}

const HTTP_TIMEOUT: Duration = Duration::from_secs(3);
const EVAL_TIMEOUT: Duration = Duration::from_secs(10);

/// `SteamClient.Apps.SetCustomArtworkForApp` asset types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Artwork {
    Grid = 0,
    Hero = 1,
    Logo = 2,
    WideGrid = 3,
}

/// A non-Steam shortcut as the running client reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientShortcut {
    pub appid: u32,
    pub app_name: String,
    pub exe: String,
    pub launch_options: String,
}

/// What [`SteamClient::add_shortcut`] creates. `exe` and `start_dir` keep the
/// quotes Steam stores (`"/path/heronite-rs"`).
#[derive(Debug, Clone)]
pub struct NewShortcut<'a> {
    pub name: &'a str,
    pub exe: &'a str,
    pub start_dir: &'a str,
    pub launch_options: &'a str,
    pub icon: Option<&'a str>,
}

/// The debugger of one Steam client.
#[derive(Debug, Clone)]
pub struct SteamClient {
    addr: String,
}

impl SteamClient {
    /// `addr` is `host:port`; the WebSocket always goes to the same address,
    /// whatever host the target list names.
    pub fn new(addr: impl Into<String>) -> Self {
        Self { addr: addr.into() }
    }

    /// `ws://` path of the `SharedJSContext` target, where `SteamClient` lives.
    fn socket_path(&self) -> Result<String> {
        let body = http_get(&self.addr, "/json")?;
        let targets: Value = serde_json::from_str(&body).context("the target list is not JSON")?;
        let url = targets
            .as_array()
            .and_then(|list| list.iter().find(|t| t.get("title").and_then(Value::as_str) == Some("SharedJSContext")))
            .and_then(|t| t.get("webSocketDebuggerUrl"))
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("Steam has no SharedJSContext target"))?;
        let rest = url.strip_prefix("ws://").ok_or_else(|| anyhow!("unexpected debugger url {url}"))?;
        Ok(rest.find('/').map(|i| rest[i..].to_string()).unwrap_or_else(|| "/".into()))
    }

    /// `isSteamClientApiAvailable`.
    pub fn is_available(&self) -> bool {
        match self.socket_path() {
            Ok(_) => true,
            Err(e) => {
                log::debug!("The Steam client debugging interface is not reachable: {e:#}");
                false
            }
        }
    }

    /// `evalInSteamClient`: `Runtime.evaluate` awaiting the promise and
    /// returning the value.
    pub fn eval(&self, expression: &str) -> Result<Value> {
        let path = self.socket_path().context(tr!("The Steam client debugging interface is not available"))?;
        let mut ws = WebSocket::connect(&self.addr, &path, EVAL_TIMEOUT).context(tr!("Connection to the Steam client failed"))?;
        let request = json!({
            "id": 1,
            "method": "Runtime.evaluate",
            "params": { "expression": expression, "awaitPromise": true, "returnByValue": true },
        });
        ws.send_text(&request.to_string())?;
        loop {
            let text =
                ws.read_message().map_err(|e| if is_timeout(&e) { anyhow!(tr!("Timed out waiting for the Steam client")) } else { e })?;
            let response: Value = serde_json::from_str(&text).context("the Steam client sent invalid JSON")?;
            if response.get("id").and_then(Value::as_i64) != Some(1) {
                continue;
            }
            ws.close();
            if let Some(err) = response.get("error") {
                bail!("{}", err.get("message").and_then(Value::as_str).unwrap_or("SteamClient call failed"));
            }
            let result = response.get("result").cloned().unwrap_or(Value::Null);
            if let Some(ex) = result.get("exceptionDetails") {
                let description = ex
                    .pointer("/exception/description")
                    .and_then(Value::as_str)
                    .or_else(|| ex.get("text").and_then(Value::as_str))
                    .unwrap_or("SteamClient call failed");
                bail!("{description}");
            }
            return Ok(result.pointer("/result/value").cloned().unwrap_or(Value::Null));
        }
    }

    /// `getSteamClientShortcuts`.
    pub fn shortcuts(&self) -> Result<Vec<ClientShortcut>> {
        let value = self.eval(
            r#"(async () => {
      const apps = appStore.allApps.filter((app) => app.BIsShortcut())
      const shortcuts = []
      for (const app of apps) {
        let details = appDetailsStore.GetAppDetails(app.appid)
        if (!details) {
          appDetailsStore.RequestAppDetails(app.appid)
          for (let i = 0; i < 10 && !details; i++) {
            await new Promise((resolve) => setTimeout(resolve, 100))
            details = appDetailsStore.GetAppDetails(app.appid)
          }
        }
        shortcuts.push({
          appid: app.appid,
          appName: details?.strDisplayName ?? app.display_name ?? '',
          exe: details?.strShortcutExe ?? '',
          launchOptions: details?.strShortcutLaunchOptions ?? ''
        })
      }
      return shortcuts
    })()"#,
        )?;
        let list = value.as_array().ok_or_else(|| anyhow!(tr!("Could not get the shortcut list from the Steam client")))?;
        let text = |v: &Value, k: &str| v.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
        Ok(list
            .iter()
            .filter_map(|s| {
                Some(ClientShortcut {
                    appid: appid_of(s.get("appid")?)?,
                    app_name: text(s, "appName"),
                    exe: text(s, "exe"),
                    launch_options: text(s, "launchOptions"),
                })
            })
            .collect())
    }

    /// `addShortcutViaSteamClient`: the app id Steam gave the shortcut.
    pub fn add_shortcut(&self, s: &NewShortcut) -> Result<u32> {
        let js = |v: &str| Value::String(v.to_string()).to_string();
        let (name, exe, start_dir, options) = (js(s.name), js(s.exe), js(s.start_dir), js(s.launch_options));
        let icon = s.icon.map(|i| format!("SteamClient.Apps.SetShortcutIcon(appId, {})", js(i))).unwrap_or_default();
        let value = self.eval(&format!(
            r#"(async () => {{
      const appId = await SteamClient.Apps.AddShortcut({name}, {exe}, '', '')
      if (typeof appId !== 'number' || appId <= 0) {{
        throw new Error('AddShortcut did not return an app id')
      }}
      SteamClient.Apps.SetShortcutName(appId, {name})
      SteamClient.Apps.SetShortcutExe(appId, {exe})
      SteamClient.Apps.SetShortcutStartDir(appId, {start_dir})
      SteamClient.Apps.SetShortcutLaunchOptions(appId, {options})
      {icon}
      return appId
    }})()"#
        ))?;
        appid_of(&value).ok_or_else(|| anyhow!("AddShortcut did not return an app id"))
    }

    /// `removeShortcutViaSteamClient`.
    pub fn remove_shortcut(&self, appid: u32) -> Result<()> {
        self.eval(&format!("(async () => {{ SteamClient.Apps.RemoveShortcut({appid}); return true }})()"))?;
        Ok(())
    }

    /// `setShortcutArtworkViaSteamClient` with the image bytes (`ext` without
    /// the dot).
    pub fn set_artwork(&self, appid: u32, data: &[u8], ext: &str, kind: Artwork) -> Result<()> {
        let data = Value::String(base64(data)).to_string();
        let ext = Value::String(ext.to_string()).to_string();
        self.eval(&format!(
            "(async () => {{ await SteamClient.Apps.SetCustomArtworkForApp({appid}, {data}, {ext}, {}); return true }})()",
            kind as u8
        ))?;
        Ok(())
    }

    /// [`set_artwork`](Self::set_artwork) from a file; the extension comes from
    /// the content (Heroic's transparent logo is a PNG named `.jpg`).
    pub fn set_artwork_file(&self, appid: u32, file: &Path, kind: Artwork) -> Result<()> {
        let data = std::fs::read(file).with_context(|| format!("reading {}", file.display()))?;
        let ext = if data.starts_with(b"\x89PNG") { "png" } else { "jpg" };
        self.set_artwork(appid, &data, ext, kind)
    }
}

/// App ids come back as JS numbers; shortcut ids have the top bit set, so a
/// negative (signed) value is the same id (`appId >>> 0`).
fn appid_of(v: &Value) -> Option<u32> {
    let n = v.as_i64().or_else(|| v.as_f64().map(|f| f as i64))?;
    (n != 0).then_some(n as u32)
}

fn is_timeout(e: &anyhow::Error) -> bool {
    e.downcast_ref::<std::io::Error>().is_some_and(|e| matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut))
}

fn connect(addr: &str, timeout: Duration) -> Result<TcpStream> {
    let sock = addr.to_socket_addrs()?.next().ok_or_else(|| anyhow!("bad address {addr}"))?;
    let stream = TcpStream::connect_timeout(&sock, timeout.min(HTTP_TIMEOUT))?;
    stream.set_read_timeout(Some(timeout))?;
    stream.set_write_timeout(Some(timeout))?;
    Ok(stream)
}

/// Status line + headers; `Content-Length` when there is one.
fn read_head(reader: &mut impl BufRead) -> Result<(String, Option<usize>)> {
    let mut status = String::new();
    reader.read_line(&mut status)?;
    let mut length = None;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 || line.trim().is_empty() {
            break;
        }
        if let Some((k, v)) = line.split_once(':') {
            if k.trim().eq_ignore_ascii_case("content-length") {
                length = v.trim().parse().ok();
            }
        }
    }
    Ok((status.trim().to_string(), length))
}

fn http_get(addr: &str, path: &str) -> Result<String> {
    let mut stream = connect(addr, HTTP_TIMEOUT)?;
    // Current clients drop HTTP/1.0 requests without an answer.
    write!(stream, "GET {path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n")?;
    let mut reader = BufReader::new(stream);
    let (status, length) = read_head(&mut reader)?;
    if status.split_whitespace().nth(1) != Some("200") {
        bail!("GET {path}: {status}");
    }
    let mut body = Vec::new();
    match length {
        Some(n) => {
            body.resize(n, 0);
            reader.read_exact(&mut body)?;
        }
        None => {
            reader.read_to_end(&mut body)?;
        }
    }
    Ok(String::from_utf8_lossy(&body).into_owned())
}

/// Random bytes for the WebSocket key and masks (not secret, only unique).
fn random_bytes<const N: usize>() -> [u8; N] {
    let mut out = [0u8; N];
    let state = std::collections::hash_map::RandomState::new();
    for (i, chunk) in out.chunks_mut(8).enumerate() {
        let mut h = state.build_hasher();
        h.write_usize(i);
        h.write_u128(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0));
        let bytes = h.finish().to_le_bytes();
        chunk.copy_from_slice(&bytes[..chunk.len()]);
    }
    out
}

/// Standard base64 with padding.
pub fn base64(data: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = (b[0] as u32) << 16 | (b[1] as u32) << 8 | b[2] as u32;
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(ALPHABET[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// Client side of RFC 6455: masked text frames out, any frames in.
struct WebSocket {
    reader: BufReader<TcpStream>,
    writer: TcpStream,
}

impl WebSocket {
    fn connect(addr: &str, path: &str, timeout: Duration) -> Result<Self> {
        let mut writer = connect(addr, timeout)?;
        let key = base64(&random_bytes::<16>());
        write!(
            writer,
            "GET {path} HTTP/1.1\r\nHost: {addr}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\n\r\n"
        )?;
        let mut reader = BufReader::new(writer.try_clone()?);
        let (status, _) = read_head(&mut reader)?;
        if status.split_whitespace().nth(1) != Some("101") {
            bail!("WebSocket handshake refused: {status}");
        }
        Ok(Self { reader, writer })
    }

    fn send_frame(&mut self, opcode: u8, payload: &[u8]) -> Result<()> {
        let mut frame = Vec::with_capacity(payload.len() + 14);
        frame.push(0x80 | opcode);
        match payload.len() {
            n if n < 126 => frame.push(0x80 | n as u8),
            n if n <= u16::MAX as usize => {
                frame.push(0x80 | 126);
                frame.extend_from_slice(&(n as u16).to_be_bytes());
            }
            n => {
                frame.push(0x80 | 127);
                frame.extend_from_slice(&(n as u64).to_be_bytes());
            }
        }
        let mask = random_bytes::<4>();
        frame.extend_from_slice(&mask);
        frame.extend(payload.iter().enumerate().map(|(i, b)| b ^ mask[i % 4]));
        self.writer.write_all(&frame)?;
        Ok(())
    }

    fn send_text(&mut self, text: &str) -> Result<()> {
        self.send_frame(0x1, text.as_bytes())
    }

    /// The next complete text (or binary) message; answers pings.
    fn read_message(&mut self) -> Result<String> {
        let mut message = Vec::new();
        loop {
            let mut head = [0u8; 2];
            self.reader.read_exact(&mut head)?;
            let (fin, opcode, masked) = (head[0] & 0x80 != 0, head[0] & 0x0f, head[1] & 0x80 != 0);
            let len = match head[1] & 0x7f {
                126 => {
                    let mut b = [0u8; 2];
                    self.reader.read_exact(&mut b)?;
                    u16::from_be_bytes(b) as u64
                }
                127 => {
                    let mut b = [0u8; 8];
                    self.reader.read_exact(&mut b)?;
                    u64::from_be_bytes(b)
                }
                n => n as u64,
            };
            if len > 256 * 1024 * 1024 {
                bail!("WebSocket frame too large ({len} bytes)");
            }
            let mut mask = [0u8; 4];
            if masked {
                self.reader.read_exact(&mut mask)?;
            }
            let mut payload = vec![0u8; len as usize];
            self.reader.read_exact(&mut payload)?;
            if masked {
                payload.iter_mut().enumerate().for_each(|(i, b)| *b ^= mask[i % 4]);
            }
            match opcode {
                0x8 => bail!(tr!("The Steam client closed the connection")),
                0x9 => self.send_frame(0xA, &payload)?,
                0xA => {}
                _ => {
                    message.extend_from_slice(&payload);
                    if fin {
                        return Ok(String::from_utf8_lossy(&message).into_owned());
                    }
                }
            }
        }
    }

    fn close(&mut self) {
        let _ = self.send_frame(0x8, &[]);
    }
}

#[cfg(test)]
pub(crate) mod testing {
    //! A fake Steam debugger: `/json` plus a WebSocket that answers every
    //! `Runtime.evaluate` with `reply(expression)`.

    use super::*;
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};

    pub type Reply = Box<dyn Fn(&str) -> Value + Send + Sync>;

    pub struct FakeSteam {
        pub addr: String,
        pub expressions: Arc<Mutex<Vec<String>>>,
    }

    fn server_frame(payload: &[u8]) -> Vec<u8> {
        let mut f = vec![0x81];
        match payload.len() {
            n if n < 126 => f.push(n as u8),
            n if n <= u16::MAX as usize => {
                f.push(126);
                f.extend_from_slice(&(n as u16).to_be_bytes());
            }
            n => {
                f.push(127);
                f.extend_from_slice(&(n as u64).to_be_bytes());
            }
        }
        f.extend_from_slice(payload);
        f
    }

    /// `reply` gets the expression; an object `{"throw": "msg"}` becomes an exception.
    pub fn start(reply: Reply) -> FakeSteam {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        let expressions = Arc::new(Mutex::new(Vec::new()));
        let seen = expressions.clone();
        let reply = Arc::new(reply);
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let (seen, reply) = (seen.clone(), reply.clone());
                std::thread::spawn(move || {
                    let mut writer = stream.try_clone().unwrap();
                    let mut reader = BufReader::new(stream);
                    let mut request = String::new();
                    reader.read_line(&mut request).unwrap();
                    let mut key_seen = false;
                    loop {
                        let mut line = String::new();
                        reader.read_line(&mut line).unwrap();
                        key_seen |= line.starts_with("Sec-WebSocket-Key");
                        if line.trim().is_empty() {
                            break;
                        }
                    }
                    if request.starts_with("GET /json ") {
                        // A non-local host: the client must keep using its own address.
                        let body = json!([
                            {"title": "Steam Big Picture Mode", "webSocketDebuggerUrl": "ws://10.9.9.9:8080/devtools/page/X"},
                            {"title": "SharedJSContext", "webSocketDebuggerUrl": "ws://10.9.9.9:8080/devtools/page/SJS"},
                        ])
                        .to_string();
                        write!(writer, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{body}", body.len()).unwrap();
                        return;
                    }
                    assert!(request.starts_with("GET /devtools/page/SJS "), "{request}");
                    assert!(key_seen);
                    writer.write_all(b"HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\r\n").unwrap();
                    let mut ws = WebSocket { reader, writer: writer.try_clone().unwrap() };
                    let Ok(text) = ws.read_message() else { return };
                    let req: Value = serde_json::from_str(&text).unwrap();
                    let expr = req.pointer("/params/expression").and_then(Value::as_str).unwrap().to_string();
                    assert_eq!(req.pointer("/params/awaitPromise"), Some(&json!(true)));
                    seen.lock().unwrap().push(expr.clone());
                    let value = reply(&expr);
                    // An unrelated event first: the client must wait for id 1.
                    writer.write_all(&server_frame(br#"{"method":"Runtime.consoleAPICalled"}"#)).unwrap();
                    let response = match value.get("throw").and_then(Value::as_str) {
                        Some(msg) => {
                            json!({"id": 1, "result": {"result": {}, "exceptionDetails": {"text": "Uncaught", "exception": {"description": msg}}}})
                        }
                        None => json!({"id": 1, "result": {"result": {"type": "object", "value": value}}}),
                    };
                    writer.write_all(&server_frame(response.to_string().as_bytes())).unwrap();
                });
            }
        });
        FakeSteam { addr, expressions }
    }
}

#[cfg(test)]
mod tests {
    use super::testing::start;
    use super::*;

    #[test]
    fn base64_matches_the_rfc_vectors() {
        let cases =
            [("", ""), ("f", "Zg=="), ("fo", "Zm8="), ("foo", "Zm9v"), ("foob", "Zm9vYg=="), ("fooba", "Zm9vYmE="), ("foobar", "Zm9vYmFy")];
        for (raw, enc) in cases {
            assert_eq!(base64(raw.as_bytes()), enc);
        }
    }

    #[test]
    fn app_ids_wrap_like_unsigned_shift() {
        assert_eq!(appid_of(&json!(3_000_000_001u64)), Some(3_000_000_001));
        assert_eq!(appid_of(&json!(-1_294_967_295i64)), Some(3_000_000_001));
        assert_eq!(appid_of(&json!(0)), None);
        assert_eq!(appid_of(&json!("x")), None);
    }

    #[test]
    fn the_port_comes_from_steamwebhelpers_command_line() {
        let dir = tempfile::tempdir().unwrap();
        let proc = dir.path();
        let add = |pid: &str, args: &[&str]| {
            std::fs::create_dir_all(proc.join(pid)).unwrap();
            std::fs::write(proc.join(pid).join("cmdline"), args.join("\0")).unwrap();
        };
        add("self", &["x"]);
        add("10", &["/usr/bin/bash", "--remote-debugging-port=1"]);
        add("11", &["/s/ubuntu12_64/steamwebhelper", "--type=zygote"]);
        assert_eq!(debugger_port(proc), None);
        add("12", &["./steamwebhelper", "-nocrashdialog", "--remote-debugging-address=127.0.0.1", "--remote-debugging-port=39445"]);
        assert_eq!(debugger_port(proc), Some(39445));
    }

    #[test]
    fn nothing_listening_means_unavailable() {
        let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let client = SteamClient::new(format!("127.0.0.1:{port}"));
        assert!(!client.is_available());
        assert!(client.eval("1").is_err());
    }

    #[test]
    fn lists_adds_removes_and_sets_artwork() {
        let fake = start(Box::new(|expr| {
            if expr.contains("appStore.allApps") {
                json!([{"appid": -1_294_967_295i64, "appName": "Control", "exe": "\"/bin/heronite-rs\"", "launchOptions": "--no-gui \"heronite://launch?appName=Calluna&runner=legendary\""}])
            } else if expr.contains("AddShortcut") {
                json!(3_123_456_789u64)
            } else {
                json!(true)
            }
        }));
        let client = SteamClient::new(&fake.addr);
        assert!(client.is_available());

        let list = client.shortcuts().unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].appid, 3_000_000_001);
        assert_eq!(list[0].app_name, "Control");

        let id = client
            .add_shortcut(&NewShortcut {
                name: "Ctrl \"Q\"",
                exe: "\"/bin/heronite-rs\"",
                start_dir: "\"/bin\"",
                launch_options: "--no-gui \"heronite://launch?appName=A&runner=sideload\"",
                icon: Some("/icons/A.png"),
            })
            .unwrap();
        assert_eq!(id, 3_123_456_789);
        client.remove_shortcut(id).unwrap();
        client.set_artwork(id, b"\x89PNGdata", "png", Artwork::Logo).unwrap();

        let seen = fake.expressions.lock().unwrap().clone();
        assert_eq!(seen.len(), 4);
        // Strings are JSON literals, so quotes in titles cannot break the script.
        assert!(seen[1].contains(r#"SteamClient.Apps.AddShortcut("Ctrl \"Q\"", "\"/bin/heronite-rs\"", '', '')"#), "{}", seen[1]);
        assert!(seen[1].contains(r#"SetShortcutIcon(appId, "/icons/A.png")"#));
        assert!(seen[2].contains("RemoveShortcut(3123456789)"));
        assert!(seen[3].contains(&format!("SetCustomArtworkForApp(3123456789, \"{}\", \"png\", 2)", base64(b"\x89PNGdata"))));
    }

    #[test]
    fn exceptions_become_errors() {
        let fake = start(Box::new(|_| json!({"throw": "Error: AddShortcut did not return an app id"})));
        let err = SteamClient::new(&fake.addr).eval("x").unwrap_err();
        assert!(format!("{err:#}").contains("AddShortcut did not return an app id"), "{err:#}");
    }

    #[test]
    fn large_payloads_use_extended_lengths() {
        let big = "x".repeat(70_000);
        let echoed = big.clone();
        let fake = start(Box::new(move |_| json!(echoed)));
        let v = SteamClient::new(&fake.addr).eval(&big).unwrap();
        assert_eq!(v.as_str().map(str::len), Some(70_000));
        assert_eq!(fake.expressions.lock().unwrap()[0].len(), 70_000);
    }
}
