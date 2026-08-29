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

/// One managed window, with the state bits a [`Region`] cannot carry.
///
/// Exists because geometry alone CANNOT distinguish a minimised window from a
/// visible one: X11 keeps a window's last geometry while it is iconified, and
/// it stays in `_NET_CLIENT_LIST` — measured live 2026-08-29 (xterm minimised:
/// still listed, geometry unchanged 184x69, `_NET_WM_STATE_HIDDEN` set). A
/// consumer that crops the screen at a bbox from this list therefore needs
/// `showing` — cropping at a hidden window's stale rectangle records whatever
/// happens to be underneath it (an FR-114 violation, silently).
#[derive(Debug, Clone)]
pub struct WmWindowState {
    /// Screen-absolute geometry. For a hidden window this is where it WAS.
    pub bbox: PixelRect,
    /// `_NET_WM_NAME`, falling back to `WM_CLASS`.
    pub label: Option<String>,
    /// Whether the window is currently showing pixels on the active desktop.
    ///
    /// False when `_NET_WM_STATE_HIDDEN` is set (minimised) OR the window's
    /// `_NET_WM_DESKTOP` is neither the current desktop nor sticky
    /// (0xFFFFFFFF). Both checks are needed: the live probe showed this WM
    /// does NOT set HIDDEN for a window on another workspace — only the
    /// desktop field moves — exactly the EWMH caveat.
    pub showing: bool,
}

impl WmProvider {
    /// Enumerate managed top-level windows with their visibility state.
    ///
    /// Unlike [`WmProvider::windows`] this does NOT filter zero-area windows —
    /// the caller decides what a degenerate geometry means for it.
    pub fn window_states() -> Result<Vec<WmWindowState>> {
        let (conn, screen_num) = x11rb::connect(None).context("connect to X11 (is DISPLAY set?)")?;
        let root = conn.setup().roots[screen_num].root;

        let net_client_list = intern(&conn, b"_NET_CLIENT_LIST")?;
        let net_wm_name = intern(&conn, b"_NET_WM_NAME")?;
        let utf8 = intern(&conn, b"UTF8_STRING")?;
        let net_wm_state = intern(&conn, b"_NET_WM_STATE")?;
        let net_wm_state_hidden = intern(&conn, b"_NET_WM_STATE_HIDDEN")?;
        let net_wm_desktop = intern(&conn, b"_NET_WM_DESKTOP")?;
        let net_current_desktop = intern(&conn, b"_NET_CURRENT_DESKTOP")?;

        // The active desktop, for the other-workspace check. Absent (a non-EWMH
        // or single-desktop WM) means the check cannot fire and every window
        // counts as on the current desktop — failing toward Visible, which
        // matches the pre-existing behaviour rather than inventing hidden-ness.
        let current_desktop: Option<u32> = conn
            .get_property(false, root, net_current_desktop, AtomEnum::CARDINAL, 0, 1)
            .ok()
            .and_then(|c| c.reply().ok())
            .and_then(|r| r.value32().and_then(|mut it| it.next()));

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
            let (ax, ay) = match conn.translate_coordinates(w, root, 0, 0).map(|c| c.reply()) {
                Ok(Ok(t)) => (t.dst_x as i32, t.dst_y as i32),
                _ => continue,
            };

            // Minimised: `_NET_WM_STATE` contains `_NET_WM_STATE_HIDDEN`.
            let hidden = conn
                .get_property(false, w, net_wm_state, AtomEnum::ATOM, 0, 64)
                .ok()
                .and_then(|c| c.reply().ok())
                .and_then(|r| r.value32().map(|it| it.collect::<Vec<Atom>>()))
                .is_some_and(|atoms| atoms.contains(&net_wm_state_hidden));

            // On another workspace: `_NET_WM_DESKTOP` set, not sticky, and not
            // the current one. Only meaningful when both sides are known.
            let elsewhere = match (current_desktop, window_desktop(&conn, w, net_wm_desktop)) {
                (Some(cur), Some(d)) => d != u32::MAX && d != cur,
                _ => false,
            };

            let bbox = PixelRect {
                x: ax.max(0) as u32,
                y: ay.max(0) as u32,
                w: geo.width as u32,
                h: geo.height as u32,
            };
            let label = wm_name(&conn, w, net_wm_name, utf8).or_else(|| wm_class(&conn, w));
            out.push(WmWindowState { bbox, label, showing: !hidden && !elsewhere });
        }
        Ok(out)
    }

    /// Enumerate managed top-level windows as window-granularity [`Region`]s.
    pub fn windows() -> Result<Vec<Region>> {
        // Built on `window_states` so there is exactly ONE enumeration path
        // (the drift between two copies is the R40 failure). The contract here
        // is unchanged: zero-area windows are skipped, and hidden windows are
        // still listed — the cascade has always seen them, and narrowing its
        // input is a separate decision from adding state to it.
        Ok(Self::window_states()?
            .into_iter()
            .filter(|s| s.bbox.w != 0 && s.bbox.h != 0)
            .map(|s| {
                let mut region = Region::new(s.bbox, Source::Wm, Granularity::Window, 0.98);
                region.label = s.label;
                region
            })
            .collect())
    }
}

/// `_NET_WM_DESKTOP` — which workspace the window is on, when the WM says.
fn window_desktop(conn: &impl Connection, w: Window, net_wm_desktop: Atom) -> Option<u32> {
    conn.get_property(false, w, net_wm_desktop, AtomEnum::CARDINAL, 0, 1)
        .ok()?
        .reply()
        .ok()?
        .value32()
        .and_then(|mut it| it.next())
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

    #[test]
    #[ignore = "live: needs an X11 DISPLAY with windows (DISPLAY=:1 cargo test --ignored)"]
    fn window_states_reports_visibility() {
        // The 2026-08-29 probe measured: a minimised xterm stays in
        // `_NET_CLIENT_LIST` with its geometry unchanged and
        // `_NET_WM_STATE_HIDDEN` set; a window moved to another workspace
        // keeps geometry, does NOT get HIDDEN, and only `_NET_WM_DESKTOP`
        // moves. This live check exercises the enumeration; minimise a window
        // by hand to watch `showing` flip.
        let ws = WmProvider::window_states().expect("enumerate window states");
        eprintln!("[live] {} managed windows:", ws.len());
        for w in &ws {
            eprintln!(
                "  {}x{}+{}+{}  showing={}  {:?}",
                w.bbox.w, w.bbox.h, w.bbox.x, w.bbox.y, w.showing, w.label
            );
        }
        assert!(!ws.is_empty(), "expected at least one managed window");
    }
}
