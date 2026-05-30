//! PV2 (T424) — real HTTP round-trip against the std::net gallery server.

use gentle_eye::preview::gallery::{bind, serve_listener};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

fn get(port: u16, path: &str, range: Option<&str>) -> (String, Vec<u8>) {
    let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
    let mut req = format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\n");
    if let Some(r) = range {
        req.push_str(&format!("Range: {r}\r\n"));
    }
    req.push_str("Connection: close\r\n\r\n");
    s.write_all(req.as_bytes()).unwrap();
    let mut buf = Vec::new();
    s.read_to_end(&mut buf).unwrap();
    let split = buf.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
    let head = String::from_utf8_lossy(&buf[..split]).into_owned();
    let body = buf[split + 4..].to_vec();
    (head, body)
}

#[test]
fn live_socket_gallery_and_range() {
    let d = tempfile::tempdir().unwrap();
    std::fs::write(d.path().join("clip.mp4"), b"0123456789").unwrap();
    let root = d.path().to_path_buf();

    let listener = bind(0).unwrap();
    let port = listener.local_addr().unwrap().port();
    let handle = std::thread::spawn(move || {
        // Short idle so the server thread exits on its own after the test.
        let _ = serve_listener(listener, root, Duration::from_millis(800));
    });
    std::thread::sleep(Duration::from_millis(50));

    // Gallery index lists the capture.
    let (head, body) = get(port, "/", None);
    assert!(head.starts_with("HTTP/1.1 200"), "head: {head}");
    assert!(String::from_utf8_lossy(&body).contains("/media/clip.mp4"));

    // Ranged media request → 206 + exact bytes.
    let (head, body) = get(port, "/media/clip.mp4", Some("bytes=2-4"));
    assert!(head.starts_with("HTTP/1.1 206"), "head: {head}");
    assert!(head.contains("Content-Range: bytes 2-4/10"));
    assert_eq!(body, b"234");

    // Path traversal rejected.
    let (head, _) = get(port, "/media/../../etc/passwd", None);
    assert!(head.starts_with("HTTP/1.1 404"), "head: {head}");

    handle.join().unwrap();
}
