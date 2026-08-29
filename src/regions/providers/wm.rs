//! `WmProvider` (E2) — window-level regions from the X11 window manager (EWMH).
//!
//! Reads `_NET_CLIENT_LIST` off the root window for the managed top-level windows,
//! then for each: `GetGeometry` (size) + `TranslateCoordinates` (absolute screen
//! position) → an exact [`Region`] at [`Granularity::Window`], labeled by
//! `_NET_WM_NAME` (falling back to `WM_CLASS`). Free, deterministic, works for ALL
//! apps regardless of toolkit — this is the "grab the browser window" primitive.
//!
//! X11 only (Wayland window geometry is compositor-dependent — out of scope for v1).

use anyhow::{Context, Result};
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{Atom, AtomEnum, ConnectionExt, Window};

use crate::regions::{Cost, Granularity, Region, RegionProvider, Source};
use crate::target::model::PixelRect;

/// Window-manager region provider (X11 EWMH). Unit struct — no state.
pub struct WmProvider;

impl WmProvider {
    /// Enumerate managed top-level windows as window-granularity [`Region`]s.
    pub fn windows() -> Result<Vec<Region>> {
        let (conn, screen_num) = x11rb::connect(None).context("connect to X11 (is DISPLAY set?)")?;
        let root = conn.setup().roots[screen_num].root;

        let net_client_list = intern(&conn, b"_NET_CLIENT_LIST")?;
        let net_wm_name = intern(&conn, b"_NET_WM_NAME")?;
        let utf8 = intern(&conn, b"UTF8_STRING")?;

        // Managed top-level windows, in stacking order.
        let list = conn
            .get_property(false, root, net_client_list, AtomEnum::WINDOW, 0, u32::MAX)?
            .reply()
            .context("read _NET_CLIENT_LIST")?;
        let windows: Vec<Window> = list.value32().map(|it| it.collect()).unwrap_or_default();

        let mut out = Vec::with_capacity(windows.len());
        for w in windows {
            // size (window-relative x/y is useless — translate to root for absolute)
            let geo = match conn.get_geometry(w).map(|c| c.reply()) {
                Ok(Ok(g)) => g,
                _ => continue, // window vanished between the list and the query
            };
            if geo.width == 0 || geo.height == 0 {
                continue;
            }
            let (ax, ay) = match conn.translate_coordinates(w, root, 0, 0).map(|c| c.reply()) {
                Ok(Ok(t)) => (t.dst_x as i32, t.dst_y as i32),
                _ => continue,
            };

            let bbox = PixelRect {
                x: ax.max(0) as u32,
                y: ay.max(0) as u32,
                w: geo.width as u32,
                h: geo.height as u32,
            };
            let label = wm_name(&conn, w, net_wm_name, utf8).or_else(|| wm_class(&conn, w));

            let mut region = Region::new(bbox, Source::Wm, Granularity::Window, 0.98);
            region.label = label;
            out.push(region);
        }
        Ok(out)
    }
}

impl RegionProvider for WmProvider {
    fn source(&self) -> Source {
        Source::Wm
    }
    fn granularity(&self) -> Granularity {
        Granularity::Window
    }
    fn cost(&self) -> Cost {
        Cost::Free
    }
    /// Windows are children of the desktop root → only answer at monitor level.
    fn probe(&self, within: &Region) -> bool {
        within.granularity == Granularity::Monitor
    }
    fn regions(&self, _within: &Region) -> Vec<Region> {
        WmProvider::windows().unwrap_or_default()
    }
}

fn intern(conn: &impl Connection, name: &[u8]) -> Result<Atom> {
    Ok(conn.intern_atom(false, name)?.reply()?.atom)
}

/// `_NET_WM_NAME` (UTF-8) — the human window title, when set.
fn wm_name(conn: &impl Connection, w: Window, net_wm_name: Atom, utf8: Atom) -> Option<String> {
    let r = conn.get_property(false, w, net_wm_name, utf8, 0, 1024).ok()?.reply().ok()?;
    if r.value.is_empty() {
        return None;
    }
    Some(String::from_utf8_lossy(&r.value).trim_end_matches('\0').to_string())
}

/// `WM_CLASS` = `"instance\0class\0"` — fall back to the class (2nd field).
fn wm_class(conn: &impl Connection, w: Window) -> Option<String> {
    let r = conn
        .get_property(false, w, AtomEnum::WM_CLASS, AtomEnum::STRING, 0, 1024)
        .ok()?
        .reply()
        .ok()?;
    if r.value.is_empty() {
        return None;
    }
    let parts: Vec<&[u8]> = r.value.split(|b| *b == 0).filter(|s| !s.is_empty()).collect();
    parts.last().map(|s| String::from_utf8_lossy(s).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "live: needs an X11 DISPLAY with windows (DISPLAY=:1 cargo test --ignored)"]
    fn lists_windows() {
        let ws = WmProvider::windows().expect("enumerate windows");
        eprintln!("[live] {} managed windows:", ws.len());
        for w in &ws {
            eprintln!(
                "  {}x{}+{}+{}  trust={:.2}  {:?}",
                w.bbox.w, w.bbox.h, w.bbox.x, w.bbox.y, w.trust, w.label
            );
            assert_eq!(w.source, Source::Wm);
            assert_eq!(w.granularity, Granularity::Window);
        }
        assert!(!ws.is_empty(), "expected at least one managed window");
    }
}
