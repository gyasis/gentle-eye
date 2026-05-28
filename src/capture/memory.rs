//! Memory-pressure monitor for the capture/encode pipeline.
//!
//! The encoder uses this to switch from streaming to file-based mode (and, at
//! the extreme, to abort with [`RecordingError::MemoryPressure`]) when the host
//! is close to exhausting RAM — mirroring the unified-memory swap-death guard
//! documented for Apple Silicon hosts.
//!
//! Reads `/proc/meminfo` on Linux (std-only, no extra dependency). On other
//! platforms it reports [`MemoryPressure::Normal`] with unknown stats so the
//! pipeline degrades gracefully rather than failing to build.
//!
//! Authored 2026-05-28 from the recovered public API
//! (`MemoryConfig, MemoryMonitor, MemoryPressure, MemoryStats`) + PRD memory
//! notes — the recovered source for this file was garbage.

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

/// Read total + available memory from `/proc/meminfo` (Linux only).
#[cfg(target_os = "linux")]
fn read_system_memory() -> Option<MemoryStats> {
    let raw = std::fs::read_to_string("/proc/meminfo").ok()?;
    parse_meminfo(&raw)
}

#[cfg(not(target_os = "linux"))]
fn read_system_memory() -> Option<MemoryStats> {
    // No std-only portable source on macOS/Windows; report unknown so callers
    // fall back to MemoryPressure::Normal rather than failing to build.
    None
}

/// Parse `MemTotal` / `MemAvailable` (in kB) from `/proc/meminfo` content.
/// Factored out so it can be unit-tested without touching the real filesystem.
fn parse_meminfo(content: &str) -> Option<MemoryStats> {
    let mut total_kb = None;
    let mut avail_kb = None;
    for line in content.lines() {
        if let Some(rest) = line.strip_prefix("MemTotal:") {
            total_kb = parse_kb(rest);
        } else if let Some(rest) = line.strip_prefix("MemAvailable:") {
            avail_kb = parse_kb(rest);
        }
        if total_kb.is_some() && avail_kb.is_some() {
            break;
        }
    }
    let total = total_kb?;
    let avail = avail_kb?;
    Some(MemoryStats {
        total_bytes: total * 1024,
        available_bytes: avail * 1024,
    })
}

/// Parse the leading integer (kB) from a `/proc/meminfo` value field.
fn parse_kb(field: &str) -> Option<u64> {
    field.split_whitespace().next()?.parse::<u64>().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
MemTotal:       16384000 kB
MemFree:         1000000 kB
MemAvailable:    4096000 kB
Buffers:          200000 kB
";

    #[test]
    fn parses_meminfo() {
        let stats = parse_meminfo(SAMPLE).unwrap();
        assert_eq!(stats.total_bytes, 16_384_000 * 1024);
        assert_eq!(stats.available_bytes, 4_096_000 * 1024);
        assert_eq!(stats.used_bytes(), (16_384_000 - 4_096_000) * 1024);
    }

    #[test]
    fn missing_fields_yield_none() {
        assert!(parse_meminfo("Buffers: 200000 kB\n").is_none());
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
