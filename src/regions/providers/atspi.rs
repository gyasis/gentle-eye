//! `AtSpiProvider` (E3) — pane/element regions from the Linux accessibility tree.
//!
//! Walks the AT-SPI2 tree over D-Bus (registry root → applications → windows →
//! elements) via the pure-Rust `atspi` crate, reading each node's
//! `Component.get_extents(Screen)` + role + name → semantic [`Region`]s. Free,
//! exact, ~zero compute — the structural ground truth that solves "the second
//! pane" without pixels, for apps that expose a tree (GTK/Qt/Electron/Firefox;
//! Chromium/VS Code/Cursor need `--force-renderer-accessibility`, and the session
//! needs `toolkit-accessibility=true` / the a11y bus running).
//!
//! X11 first. Coverage is app-dependent (the consumer, e.g. Lookout, owns the
//! "enable accessibility" UX; the engine just reports what the tree exposes).

use std::time::Duration;

use anyhow::Result;
use atspi::connection::AccessibilityConnection;
use atspi::proxy::accessible::{AccessibleProxy, ObjectRefExt};
use atspi::proxy::proxy_ext::ProxyExt;
use atspi::zbus;
use atspi::CoordType;

use crate::regions::{Cost, Granularity, Region, RegionProvider, Source};
use crate::target::model::PixelRect;

const MAX_DEPTH: usize = 10;
const MAX_NODES: usize = 4000;
const WALK_TIMEOUT: Duration = Duration::from_secs(4);

/// Accessibility-tree region provider (AT-SPI2 over D-Bus). Unit struct.
pub struct AtSpiProvider;

impl AtSpiProvider {
    /// Walk the accessibility tree → element/pane [`Region`]s (role + name + box).
    pub async fn elements() -> Result<Vec<Region>> {
        let conn = AccessibilityConnection::new().await?;
        let dbus = conn.connection();

        // The registry daemon's root Accessible; its children are the running apps.
        let root = AccessibleProxy::builder(dbus)
            .destination("org.a11y.atspi.Registry")?
            .path("/org/a11y/atspi/accessible/root")?
            .cache_properties(zbus::proxy::CacheProperties::No)
            .build()
            .await?;

        let mut out = Vec::new();
        let mut budget = MAX_NODES;
        for app in root.get_children().await.unwrap_or_default() {
            if budget == 0 {
                break;
            }
            if let Ok(app_proxy) = app.as_accessible_proxy(dbus).await {
                Box::pin(walk(&app_proxy, dbus, 0, &mut out, &mut budget)).await;
            }
        }
        Ok(out)
    }

    /// Blocking wrapper (bounded by [`WALK_TIMEOUT`]) — runs the async walk on an
    /// isolated current-thread runtime so it works whether or not the caller is
    /// already inside a tokio runtime. Returns `[]` on any failure/timeout.
    fn elements_blocking() -> Vec<Region> {
        std::thread::spawn(|| {
            let rt = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
                Ok(rt) => rt,
                Err(_) => return Vec::new(),
            };
            rt.block_on(async {
                tokio::time::timeout(WALK_TIMEOUT, AtSpiProvider::elements())
                    .await
                    .ok()
                    .and_then(|r| r.ok())
                    .unwrap_or_default()
            })
        })
        .join()
        .unwrap_or_default()
    }
}

impl RegionProvider for AtSpiProvider {
    fn source(&self) -> Source {
        Source::AtSpi
    }
    fn granularity(&self) -> Granularity {
        Granularity::Element // spans pane+element; reports the finest it reaches
    }
    fn cost(&self) -> Cost {
        Cost::Free
    }
    fn probe(&self, within: &Region) -> bool {
        matches!(within.granularity, Granularity::Monitor | Granularity::Window)
    }
    fn regions(&self, _within: &Region) -> Vec<Region> {
        AtSpiProvider::elements_blocking()
    }
}

/// Recurse an accessible subtree, emitting a Region for any node with a
/// `Component` interface + on-screen extents. Bounded by depth + a node budget.
async fn walk(
    proxy: &AccessibleProxy<'_>,
    dbus: &zbus::Connection,
    depth: usize,
    out: &mut Vec<Region>,
    budget: &mut usize,
) {
    if depth > MAX_DEPTH || *budget == 0 {
        return;
    }
    *budget -= 1;

    let role_name = proxy.get_role_name().await.unwrap_or_default();
    if let Ok(bbox) = component_extents(proxy).await {
        if bbox.w > 0 && bbox.h > 0 {
            let label = proxy.name().await.ok().filter(|s| !s.is_empty());
            let mut r = Region::new(bbox, Source::AtSpi, granularity_for(&role_name), 0.95);
            r.role = (!role_name.is_empty()).then_some(role_name);
            r.label = label;
            out.push(r);
        }
    }

    for child in proxy.get_children().await.unwrap_or_default() {
        if *budget == 0 {
            break;
        }
        if let Ok(cp) = child.as_accessible_proxy(dbus).await {
            Box::pin(walk(&cp, dbus, depth + 1, out, budget)).await;
        }
    }
}

/// Screen-coordinate extents of a node's `Component` interface, if it has one.
async fn component_extents(proxy: &AccessibleProxy<'_>) -> Result<PixelRect> {
    let comp = proxy.proxies().await?.component().await?;
    let (x, y, w, h) = comp.get_extents(CoordType::Screen).await?;
    Ok(PixelRect {
        x: x.max(0) as u32,
        y: y.max(0) as u32,
        w: w.max(0) as u32,
        h: h.max(0) as u32,
    })
}

/// Map an AT-SPI role name → our granularity (window / pane / element).
fn granularity_for(role_name: &str) -> Granularity {
    match role_name {
        "frame" | "window" | "dialog" | "application" => Granularity::Window,
        "panel" | "scroll pane" | "split pane" | "layered pane" | "filler" | "tool bar"
        | "menu bar" | "status bar" | "page tab" | "page tab list" | "viewport" => Granularity::Pane,
        "text" | "label" | "static" | "paragraph" => Granularity::Text,
        _ => Granularity::Element,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore = "live: needs the a11y bus + toolkit-accessibility + apps exposing trees"]
    async fn walks_tree() {
        let els = tokio::time::timeout(WALK_TIMEOUT + Duration::from_secs(2), AtSpiProvider::elements())
            .await
            .expect("walk did not time out")
            .expect("walk atspi tree");
        eprintln!("[live] {} accessible regions", els.len());
        for r in els.iter().take(40) {
            eprintln!(
                "  {}x{}+{}+{}  role={:?} label={:?}",
                r.bbox.w, r.bbox.h, r.bbox.x, r.bbox.y, r.role, r.label
            );
            assert_eq!(r.source, Source::AtSpi);
        }
    }
}
