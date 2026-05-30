//! Memory-pressure monitor for the capture/encode pipeline.
//!
//! The encoder uses this to switch from streaming to file-based mode (and, at
//! the extreme, to abort with [`RecordingError::MemoryPressure`]) when the host
//! is close to exhausting RAM — mirroring the unified-memory swap-death guard
//! documented for Apple Silicon hosts.
//!
//! Reads system memory via the cross-platform `sysinfo` crate, so the monitor
//! works on Linux, macOS, and Windows (the original Linux-only `/proc/meminfo`
//! reader degraded to "unknown" everywhere else).
//!
//! Authored 2026-05-28 from the recovered public API
//! (`MemoryConfig, MemoryMonitor, MemoryPressure, MemoryStats`) + PRD memory
//! notes — the recovered source for this file was garbage. Made cross-platform
//! 2026-05-30 (sysinfo).

/// A point-in-time snapshot of system memory.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MemoryStats {
    /// Total physical memory in bytes.
    pub total_bytes: u64,
    /// Memory available for new allocations in bytes.
    pub available_bytes: u64,
}

impl MemoryStats {
    /// Bytes currently in use (`total - available`).
    pub fn used_bytes(&self) -> u64 {
        self.total_bytes.saturating_sub(self.available_bytes)
    }

    /// Fraction of memory in use, in `[0.0, 1.0]`. Returns `0.0` if total is unknown.
    pub fn usage_ratio(&self) -> f64 {
        if self.total_bytes == 0 {
            return 0.0;
        }
        self.used_bytes() as f64 / self.total_bytes as f64
    }
}

/// Discrete memory-pressure level derived from [`MemoryStats`] and thresholds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryPressure {
    /// Plenty of headroom — stream frames directly to the encoder.
    Normal,
    /// Getting tight — prefer file-based encoding to bound resident memory.
    Warning,
    /// Near exhaustion — recording should abort to avoid swap death / OOM.
    Critical,
}

/// Thresholds (as usage ratios) that map [`MemoryStats`] to [`MemoryPressure`].
#[derive(Debug, Clone, Copy)]
pub struct MemoryConfig {
    /// Usage ratio at or above which pressure is [`MemoryPressure::Warning`].
    pub warning_ratio: f64,
    /// Usage ratio at or above which pressure is [`MemoryPressure::Critical`].
    pub critical_ratio: f64,
}

impl Default for MemoryConfig {
    fn default() -> Self {
        // Apple-Silicon unified memory pages to swap well before a hard OOM, so
        // we treat 80% as a warning and 90% as critical.
        Self {
            warning_ratio: 0.80,
            critical_ratio: 0.90,
        }
    }
}

/// Monitors system memory and reports pressure for the capture pipeline.
#[derive(Debug, Clone, Default)]
pub struct MemoryMonitor {
    config: MemoryConfig,
}

impl MemoryMonitor {
    /// Create a monitor with the given thresholds.
    pub fn new(config: MemoryConfig) -> Self {
        Self { config }
    }

    /// Read the current system memory stats, if available on this platform.
    pub fn current_stats(&self) -> Option<MemoryStats> {
        read_system_memory()
    }

    /// Classify the supplied stats against this monitor's thresholds.
    pub fn classify(&self, stats: &MemoryStats) -> MemoryPressure {
        let ratio = stats.usage_ratio();
        if ratio >= self.config.critical_ratio {
            MemoryPressure::Critical
        } else if ratio >= self.config.warning_ratio {
            MemoryPressure::Warning
        } else {
            MemoryPressure::Normal
        }
    }

    /// Current pressure level. Defaults to [`MemoryPressure::Normal`] when stats
    /// can't be read (e.g. on a non-Linux host).
    pub fn pressure(&self) -> MemoryPressure {
        self.current_stats()
            .map(|s| self.classify(&s))
            .unwrap_or(MemoryPressure::Normal)
    }

    /// True when pressure is [`MemoryPressure::Warning`] or worse.
    pub fn is_under_pressure(&self) -> bool {
        !matches!(self.pressure(), MemoryPressure::Normal)
    }
}

/// Read total + available physical memory (bytes) via `sysinfo` — works on
/// Linux, macOS, and Windows. Returns `None` if the platform reports no total
/// (callers then fall back to [`MemoryPressure::Normal`]).
fn read_system_memory() -> Option<MemoryStats> {
    use sysinfo::System;
    // Refresh memory only — we don't touch processes/CPU/disks, so this stays
    // cheap even if the monitor is polled repeatedly.
    let mut sys = System::new();
    sys.refresh_memory();
    let total_bytes = sys.total_memory(); // bytes (sysinfo >= 0.30)
    if total_bytes == 0 {
        return None;
    }
    Some(MemoryStats {
        total_bytes,
        available_bytes: sys.available_memory(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_real_system_memory() {
        // sysinfo reports memory on every supported host (Linux/macOS/Windows),
        // so this returns sane stats on the build machine.
        let stats = read_system_memory().expect("sysinfo should report memory");
        assert!(stats.total_bytes > 0, "total memory must be positive");
        assert!(
            stats.available_bytes <= stats.total_bytes,
            "available ({}) must not exceed total ({})",
            stats.available_bytes,
            stats.total_bytes
        );
    }

    #[test]
    fn classifies_pressure_against_thresholds() {
        let m = MemoryMonitor::default();
        let normal = MemoryStats {
            total_bytes: 100,
            available_bytes: 50,
        }; // 50% used
        let warning = MemoryStats {
            total_bytes: 100,
            available_bytes: 15,
        }; // 85% used
        let critical = MemoryStats {
            total_bytes: 100,
            available_bytes: 5,
        }; // 95% used
        assert_eq!(m.classify(&normal), MemoryPressure::Normal);
        assert_eq!(m.classify(&warning), MemoryPressure::Warning);
        assert_eq!(m.classify(&critical), MemoryPressure::Critical);
    }

    #[test]
    fn usage_ratio_handles_unknown_total() {
        let unknown = MemoryStats {
            total_bytes: 0,
            available_bytes: 0,
        };
        assert_eq!(unknown.usage_ratio(), 0.0);
    }
}
