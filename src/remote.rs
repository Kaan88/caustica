//! Phone remote control (roadmap 5d): a small HTTP server (std only) serving a page of controls.
//! Off by default; every request needs the session's random token, which the app shows with
//! the URL as a QR code. Edits arrive as `key=json` and go through `Settings::validate` like
//! `--set`, limited to the keys in `KEYS`.
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};

/// Settings the page may change, plus "preset" (an index) and "restart".
pub const KEYS: [&str; 15] = [
    "exposure",
    "tonemap",
    "view",
    "light_intensity",
    "light_temperature",
    "rotate",
    "rotate_speed",
    "yaw",
    "pitch",
    "distance",
    "fov",
    "aperture",
    "focus_distance",
    "sky",
    "denoiser",
];
pub struct Remote {
    pub url: String,
    pub token: String,
    pub edits: Receiver<(String, serde_json::Value)>,
    /// The values the page shows, refreshed by the app.
    pub state: Arc<Mutex<serde_json::Value>>,
    /// Cleared when the remote is turned off: the server then refuses every request.
    alive: Arc<AtomicBool>,
}
impl Drop for Remote {
    fn drop(&mut self) {
        self.alive.store(false, Ordering::Relaxed);
    }
}
fn token() -> String {
    let t = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64
        ^ (std::process::id() as u64) << 32;
    let mut h = t.wrapping_mul(0x9E37_79B9_7F4A_7C15);
    (0..8)
        .map(|_| {
            h ^= h >> 29;
            h = h.wrapping_mul(0xBF58_476D_1CE4_E5B9);
            b"abcdefghjkmnpqrstuvwxyz23456789"[(h % 31) as usize] as char
        })
        .collect()
}
/// This machine's LAN address (no packet is sent: connecting a UDP socket only picks a route).
fn lan_ip() -> String {
    std::net::UdpSocket::bind("0.0.0.0:0")
        .and_then(|s| {
            s.connect("192.168.0.1:9")?;
            s.local_addr()
        })
        .map(|a| a.ip().to_string())
        .unwrap_or_else(|_| "127.0.0.1".into())
}
/// Starts serving on the LAN (or only on this machine for tests) at the first free port from 8642.
pub fn start(lan: bool) -> std::io::Result<Remote> {
    let host = if lan { "0.0.0.0" } else { "127.0.0.1" };
    let listener = (8642..8662)
        .find_map(|p| TcpListener::bind((host, p)).ok())
        .ok_or_else(|| std::io::Error::other("no free port in 8642..8662"))?;
    let port = listener.local_addr()?.port();
    let token = token();
    let (send, edits) = channel();
    let state = Arc::new(Mutex::new(serde_json::json!({})));
    let ip = if lan { lan_ip() } else { "127.0.0.1".into() };
    let alive = Arc::new(AtomicBool::new(true));
    let (t, s, a) = (token.clone(), state.clone(), alive.clone());
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            if !a.load(Ordering::Relaxed) {
                let _ = (&stream).write_all(
                    b"HTTP/1.1 410 Gone\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                );
                continue;
            }
            let (t, s, send) = (t.clone(), s.clone(), send.clone());
            std::thread::spawn(move || {
                let _ = serve(stream, &t, &s, &send);
            });
        }
    });
    Ok(Remote {
        url: format!("http://{ip}:{port}/?t={token}"),
        token,
        edits,
        state,
        alive,
    })
}
fn serve(
    stream: TcpStream,
    token: &str,
    state: &Mutex<serde_json::Value>,
    send: &Sender<(String, serde_json::Value)>,
) -> std::io::Result<()> {
    stream.set_read_timeout(Some(std::time::Duration::from_secs(5)))?;
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    let mut parts = line.split_whitespace();
    let (method, target) = (parts.next().unwrap_or(""), parts.next().unwrap_or("/"));
    let mut length = 0usize;
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header)? == 0 || header.trim().is_empty() {
            break;
        }
        if let Some((k, v)) = header.split_once(':')
            && k.eq_ignore_ascii_case("content-length")
        {
            length = v.trim().parse().unwrap_or(0).min(4096);
        }
    }
    let mut body = vec![0; length];
    reader.read_exact(&mut body)?;
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    let param = |name: &str| {
        query
            .split('&')
            .find_map(|kv| kv.strip_prefix(&format!("{name}=")).map(str::to_string))
    };
    let mut out = stream;
    let reply = |out: &mut TcpStream, status: &str, kind: &str, body: &[u8]| {
        let head = format!(
            "HTTP/1.1 {status}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
            body.len()
        );
        out.write_all(head.as_bytes())?;
        out.write_all(body)
    };
    if param("t").as_deref() != Some(token) {
        return reply(
            &mut out,
            "403 Forbidden",
            "text/plain",
            b"Wrong or missing token",
        );
    }
    match (method, path) {
        ("GET", "/") => reply(
            &mut out,
            "200 OK",
            "text/html; charset=utf-8",
            PAGE.as_bytes(),
        ),
        ("GET", "/state") => {
            let json = state.lock().map(|s| s.to_string()).unwrap_or_default();
            reply(&mut out, "200 OK", "application/json", json.as_bytes())
        }
        ("POST", "/set") => {
            let key = param("k").unwrap_or_default();
            match serde_json::from_slice::<serde_json::Value>(&body) {
                Ok(v) if KEYS.contains(&key.as_str()) || key == "preset" || key == "restart" => {
                    let _ = send.send((key, v));
                    reply(&mut out, "204 No Content", "text/plain", b"")
                }
                _ => reply(
                    &mut out,
                    "400 Bad Request",
                    "text/plain",
                    b"Unknown key or bad value",
                ),
            }
        }
        _ => reply(&mut out, "404 Not Found", "text/plain", b"Not found"),
    }
}
/// A QR code of `text` as an egui image (black on white, 4-module quiet zone, 4 px per module).
pub fn qr_image(text: &str) -> Option<egui::ColorImage> {
    let code = qrcode::QrCode::new(text.as_bytes()).ok()?;
    let n = code.width();
    let (quiet, scale) = (4, 4);
    let size = (n + 2 * quiet) * scale;
    let colors = code.to_colors();
    let pixels = (0..size * size)
        .map(|i| {
            let (x, y) = ((i % size) / scale, (i / size) / scale);
            let dark = x >= quiet
                && y >= quiet
                && x < n + quiet
                && y < n + quiet
                && colors[(y - quiet) * n + x - quiet] == qrcode::Color::Dark;
            if dark {
                egui::Color32::BLACK
            } else {
                egui::Color32::WHITE
            }
        })
        .collect();
    Some(egui::ColorImage {
        size: [size, size],
        pixels,
    })
}
const PAGE: &str = r#"<!doctype html><html><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>Caustica remote</title><style>
body{font-family:system-ui,sans-serif;background:#16181c;color:#e8e8e8;margin:0;padding:16px;max-width:520px}
h1{font-size:18px;color:#60cad8;margin:0 0 12px}label{display:block;margin:14px 0 4px;font-size:14px}
input[type=range]{width:100%}button,select{font-size:16px;padding:8px 14px;margin:4px 4px 0 0;background:#2a2e35;color:#eee;border:1px solid #444;border-radius:6px}
.row{display:flex;gap:8px;align-items:center}.v{color:#9ab;font-size:13px}</style></head><body>
<h1>Caustica</h1>
<div class="row"><button onclick="send('preset',-1)">&#9664; Scene</button><button onclick="send('preset',1)">Scene &#9654;</button><button onclick="send('restart',true)">Restart</button></div>
<label>Exposure <span class="v" id="ev"></span></label><input type="range" min="-6" max="3.3" step="0.1" id="exposure" oninput="ev.textContent=this.value+' EV';send('exposure',Math.pow(2,+this.value))">
<label>Light intensity <span class="v" id="li"></span></label><input type="range" min="0" max="7.6" step="0.05" id="light_intensity" oninput="li.textContent=Math.exp(this.value).toFixed(1)+'x';send('light_intensity',Math.exp(+this.value))">
<label>Colour temperature <span class="v" id="lt"></span></label><input type="range" min="1000" max="12000" step="100" id="light_temperature" oninput="lt.textContent=this.value+' K';send('light_temperature',+this.value)">
<label>Orbit</label><input type="range" min="-180" max="180" step="1" id="yaw" oninput="send('yaw',+this.value)">
<label>Height</label><input type="range" min="-60" max="85" step="1" id="pitch" oninput="send('pitch',+this.value)">
<label>Zoom</label><input type="range" min="-0.7" max="5.3" step="0.02" id="distance" oninput="send('distance',Math.exp(+this.value))">
<label>Aperture</label><input type="range" min="0" max="0.5" step="0.005" id="aperture" oninput="send('aperture',+this.value)">
<div class="row"><label><input type="checkbox" id="rotate" onchange="send('rotate',this.checked)"> Auto orbit</label>
<select id="tonemap" onchange="send('tonemap',+this.value)"><option value=0>Linear</option><option value=1>Reinhard</option><option value=2>Reinhard white</option><option value=3>ACES</option><option value=4>Uncharted 2</option><option value=5>High contrast</option><option value=6>AgX</option><option value=7>PBR Neutral</option></select></div>
<script>
const t=new URLSearchParams(location.search).get('t');let busy={};
function send(k,v){busy[k]=Date.now();fetch('/set?t='+t+'&k='+k,{method:'POST',body:JSON.stringify(v)});}
async function sync(){try{const s=await (await fetch('/state?t='+t)).json();
for(const[k,v] of Object.entries(s)){const e=document.getElementById(k);if(!e||Date.now()-(busy[k]||0)<3000)continue;
if(e.type=='checkbox')e.checked=v;else if(k=='exposure')e.value=Math.log2(Math.max(v,0.016)).toFixed(1);
else if(k=='light_intensity')e.value=Math.log(Math.max(v,0.01));else if(k=='distance')e.value=Math.log(v);else e.value=v;}}catch(e){}}
sync();setInterval(sync,2000);
</script></body></html>"#;

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    #[test]
    fn edits_need_the_token_and_a_known_key() {
        let r = start(false).unwrap();
        let port = r
            .url
            .split(':')
            .nth(2)
            .unwrap()
            .split('/')
            .next()
            .unwrap()
            .to_string();
        let ask = |request: String| {
            let mut s = TcpStream::connect(("127.0.0.1", port.parse::<u16>().unwrap())).unwrap();
            s.write_all(request.as_bytes()).unwrap();
            let mut out = String::new();
            s.read_to_string(&mut out).unwrap();
            out
        };
        assert!(ask("GET /?t=wrong HTTP/1.1\r\n\r\n".into()).starts_with("HTTP/1.1 403"));
        assert!(ask(format!("GET /?t={} HTTP/1.1\r\n\r\n", r.token)).contains("Caustica"));
        assert!(
            ask(format!(
                "POST /set?t={}&k=gpu_hack HTTP/1.1\r\nContent-Length: 1\r\n\r\n1",
                r.token
            ))
            .starts_with("HTTP/1.1 400")
        );
        assert!(
            ask(format!(
                "POST /set?t={}&k=exposure HTTP/1.1\r\nContent-Length: 3\r\n\r\n2.5",
                r.token
            ))
            .starts_with("HTTP/1.1 204")
        );
        let (k, v) = r
            .edits
            .recv_timeout(std::time::Duration::from_secs(2))
            .unwrap();
        assert_eq!((k.as_str(), v.as_f64()), ("exposure", Some(2.5)));
        assert!(qr_image(&r.url).is_some());
    }
}
