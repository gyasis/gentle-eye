//! PV2 — a zero-dependency media gallery over `std::net`.
//!
//! Hand-rolled GET-only HTTP server (no `tiny_http`): serves a single embedded
//! gallery page at `/` and capture files at `/media/<name>`, with **HTTP Range
//! → 206** so `<video>` scrubs (Safari/iOS need this). Binds **127.0.0.1 only**;
//! path-traversal-safe; idle self-shutdown.

use super::discover::{recent_captures, Capture, CaptureKind};
use super::errors::PreviewError;
use std::io::{BufRead, BufReader, Write};
use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// A built HTTP response.
pub struct HttpResponse {
    pub status: u16,
    pub reason: &'static str,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl HttpResponse {
    fn text(status: u16, reason: &'static str, body: &str) -> Self {
        Self {
            status,
            reason,
            headers: vec![("Content-Type".into(), "text/plain; charset=utf-8".into())],
            body: body.as_bytes().to_vec(),
        }
    }
}

/// Minimal percent-decode (enough for filenames).
fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Resolve `/media/<name>` to a real file strictly under `root`, or `None` if it
/// escapes the root or doesn't exist. Defeats `..`, absolute paths, symlink-out.
pub fn resolve_media(root: &Path, req_path: &str) -> Option<PathBuf> {
    let name = req_path.strip_prefix("/media/")?;
    let name = percent_decode(name);
    if name.is_empty() || name.contains("..") || name.contains('\0') {
        return None;
    }
    if Path::new(&name).is_absolute() {
        return None;
    }
    let canon_root = root.canonicalize().ok()?;
    let canon = canon_root.join(&name).canonicalize().ok()?;
    if !canon.starts_with(&canon_root) || !canon.is_file() {
        return None;
    }
    Some(canon)
}

/// Parse a `Range:` value against total length → inclusive `(start, end)`.
pub fn parse_range(value: &str, total: u64) -> Option<(u64, u64)> {
    if total == 0 {
        return None;
    }
    let spec = value.trim().strip_prefix("bytes=")?;
    let (a, b) = spec.split_once('-')?;
    if a.is_empty() {
        // suffix range: bytes=-N → last N bytes
        let n: u64 = b.parse().ok()?;
        if n == 0 {
            return None;
        }
        let n = n.min(total);
        Some((total - n, total - 1))
    } else {
        let start: u64 = a.parse().ok()?;
        let end = if b.is_empty() {
            total - 1
        } else {
            b.parse::<u64>().ok()?.min(total - 1)
        };
        if start > end || start >= total {
            return None;
        }
        Some((start, end))
    }
}

fn content_type(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()).as_deref() {
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("webp") => "image/webp",
        Some("gif") => "image/gif",
        Some("bmp") => "image/bmp",
        Some("mp4") | Some("m4v") => "video/mp4",
        Some("webm") => "video/webm",
        Some("mkv") => "video/x-matroska",
        Some("mov") => "video/quicktime",
        _ => "application/octet-stream",
    }
}

/// Build the embedded single-page gallery HTML for `caps`.
pub fn gallery_html(caps: &[Capture]) -> String {
    let mut tiles = String::new();
    for c in caps {
        let name = c.path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        let url = format!("/media/{name}");
        let tile = match c.kind {
            CaptureKind::Image => format!(
                "<figure><img src=\"{url}\" loading=\"lazy\"><figcaption>{name}</figcaption></figure>"
            ),
            CaptureKind::Video => format!(
                "<figure><video src=\"{url}\" controls preload=\"metadata\"></video><figcaption>{name}</figcaption></figure>"
            ),
        };
        tiles.push_str(&tile);
    }
    if caps.is_empty() {
        tiles.push_str("<p>No captures found.</p>");
    }
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>gentle-eye preview</title>\
<style>body{{background:#111;color:#eee;font:14px system-ui;margin:0;padding:16px}}\
h1{{font-size:15px;font-weight:600;color:#9ad}}\
.grid{{display:grid;grid-template-columns:repeat(auto-fill,minmax(280px,1fr));gap:14px}}\
figure{{margin:0;background:#1b1b1b;border:1px solid #333;border-radius:8px;overflow:hidden}}\
img,video{{width:100%;display:block;background:#000}}\
figcaption{{padding:6px 8px;font-size:12px;color:#aaa;word-break:break-all}}</style></head>\
<body><h1>gentle-eye — {} capture(s)</h1><div class=\"grid\">{}</div></body></html>",
        caps.len(),
        tiles
    )
}

/// Handle a GET for `path` (with optional Range), serving from `root`.
pub fn handle_get(root: &Path, path: &str, range: Option<&str>) -> HttpResponse {
    let route = path.split('?').next().unwrap_or(path);
    if route == "/" || route == "/index.html" {
        let caps = recent_captures(root, 200).unwrap_or_default();
        return HttpResponse {
            status: 200,
            reason: "OK",
            headers: vec![("Content-Type".into(), "text/html; charset=utf-8".into())],
            body: gallery_html(&caps).into_bytes(),
        };
    }
    if route.starts_with("/media/") {
        let file = match resolve_media(root, route) {
            Some(f) => f,
            None => return HttpResponse::text(404, "Not Found", "not found"),
        };
        let bytes = match std::fs::read(&file) {
            Ok(b) => b,
            Err(_) => return HttpResponse::text(404, "Not Found", "not found"),
        };
        let total = bytes.len() as u64;
        let ct = content_type(&file).to_string();
        if let Some((start, end)) = range.and_then(|r| parse_range(r, total)) {
            let slice = bytes[start as usize..=end as usize].to_vec();
            return HttpResponse {
                status: 206,
                reason: "Partial Content",
                headers: vec![
                    ("Content-Type".into(), ct),
                    ("Accept-Ranges".into(), "bytes".into()),
                    ("Content-Range".into(), format!("bytes {start}-{end}/{total}")),
                ],
                body: slice,
            };
        }
        return HttpResponse {
            status: 200,
            reason: "OK",
            headers: vec![
                ("Content-Type".into(), ct),
                ("Accept-Ranges".into(), "bytes".into()),
            ],
            body: bytes,
        };
    }
    HttpResponse::text(404, "Not Found", "not found")
}

/// True when running inside an SSH session (native windows can't render there).
pub fn is_ssh_session() -> bool {
    std::env::var_os("SSH_TTY").is_some() || std::env::var_os("SSH_CLIENT").is_some()
}

/// Announce the gallery URL: locally, open the browser; over SSH, print the
/// tunnel hint instead. Returns the URL. (Messages go to stderr; stdout is JSON.)
pub fn announce(port: u16, ssh: bool) -> String {
    let url = format!("http://127.0.0.1:{port}/");
    if ssh {
        eprintln!(
            "Remote session — tunnel then open in a local browser:\n  ssh -L {port}:127.0.0.1:{port} <this-host>\n  {url}"
        );
    } else {
        let opener = if cfg!(target_os = "macos") { "open" } else { "xdg-open" };
        let _ = std::process::Command::new(opener).arg(&url).spawn();
        eprintln!("Preview gallery: {url}");
    }
    url
}

/// Bind a GET server to `127.0.0.1:port` (port 0 = ephemeral). Returns the listener.
pub fn bind(port: u16) -> Result<TcpListener, PreviewError> {
    TcpListener::bind((Ipv4Addr::LOCALHOST, port)).map_err(|e| PreviewError::Http(e.to_string()))
}

/// Serve `root` on `listener` until no request arrives for `idle`.
pub fn serve_listener(listener: TcpListener, root: PathBuf, idle: Duration) -> Result<(), PreviewError> {
    listener.set_nonblocking(true).map_err(|e| PreviewError::Http(e.to_string()))?;
    let mut last = Instant::now();
    loop {
        match listener.accept() {
            Ok((stream, _)) => {
                last = Instant::now();
                let _ = handle_conn(stream, &root);
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                if last.elapsed() >= idle {
                    return Ok(());
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => return Err(PreviewError::Http(e.to_string())),
        }
    }
}

fn handle_conn(mut stream: TcpStream, root: &Path) -> std::io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut request_line = String::new();
    reader.read_line(&mut request_line)?;
    let mut path = String::new();
    if let Some(p) = request_line.split_whitespace().nth(1) {
        path = p.to_string();
    }
    // Read headers, capture Range.
    let mut range: Option<String> = None;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 || line == "\r\n" || line == "\n" {
            break;
        }
        if let Some((name, v)) = line.split_once(':') {
            if name.eq_ignore_ascii_case("range") {
                range = Some(v.trim().to_string());
            }
        }
    }
    let resp = handle_get(root, &path, range.as_deref());
    write_response(&mut stream, &resp)
}

fn write_response(stream: &mut TcpStream, resp: &HttpResponse) -> std::io::Result<()> {
    let mut head = format!("HTTP/1.1 {} {}\r\n", resp.status, resp.reason);
    for (k, v) in &resp.headers {
        head.push_str(&format!("{k}: {v}\r\n"));
    }
    head.push_str(&format!("Content-Length: {}\r\n", resp.body.len()));
    head.push_str("Connection: close\r\n\r\n");
    stream.write_all(head.as_bytes())?;
    stream.write_all(&resp.body)?;
    stream.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn range_math() {
        assert_eq!(parse_range("bytes=0-99", 1000), Some((0, 99)));
        assert_eq!(parse_range("bytes=100-", 1000), Some((100, 999)));
        assert_eq!(parse_range("bytes=-50", 1000), Some((950, 999)));
        assert_eq!(parse_range("bytes=900-100000", 1000), Some((900, 999))); // clamped
        assert_eq!(parse_range("bytes=2000-3000", 1000), None); // past end
        assert_eq!(parse_range("bytes=0-0", 0), None); // empty file
    }

    #[test]
    fn path_traversal_rejected() {
        let d = tempfile::tempdir().unwrap();
        std::fs::File::create(d.path().join("ok.png")).unwrap().write_all(b"x").unwrap();
        assert!(resolve_media(d.path(), "/media/ok.png").is_some());
        assert!(resolve_media(d.path(), "/media/../../etc/passwd").is_none());
        assert!(resolve_media(d.path(), "/media/%2e%2e/%2e%2e/etc/passwd").is_none());
        assert!(resolve_media(d.path(), "/media/").is_none());
        assert!(resolve_media(d.path(), "/media/nope.png").is_none());
    }

    #[test]
    fn handle_get_serves_gallery_and_media_with_range() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("clip.mp4"), b"0123456789").unwrap();

        let root = d.path();
        let idx = handle_get(root, "/", None);
        assert_eq!(idx.status, 200);
        assert!(String::from_utf8_lossy(&idx.body).contains("/media/clip.mp4"));

        let full = handle_get(root, "/media/clip.mp4", None);
        assert_eq!(full.status, 200);
        assert_eq!(full.body, b"0123456789");
        assert!(full.headers.iter().any(|(k, v)| k == "Accept-Ranges" && v == "bytes"));

        let part = handle_get(root, "/media/clip.mp4", Some("bytes=2-4"));
        assert_eq!(part.status, 206);
        assert_eq!(part.body, b"234");
        assert!(part.headers.iter().any(|(k, v)| k == "Content-Range" && v == "bytes 2-4/10"));

        assert_eq!(handle_get(root, "/media/../x", None).status, 404);
    }

    #[test]
    fn gallery_html_picks_video_vs_image() {
        let d = tempfile::tempdir().unwrap();
        let caps = vec![
            Capture { path: d.path().join("a.png"), kind: CaptureKind::Image, modified: std::time::SystemTime::UNIX_EPOCH },
            Capture { path: d.path().join("b.mp4"), kind: CaptureKind::Video, modified: std::time::SystemTime::UNIX_EPOCH },
        ];
        let html = gallery_html(&caps);
        assert!(html.contains("<img src=\"/media/a.png\""));
        assert!(html.contains("<video src=\"/media/b.mp4\""));
    }
}
