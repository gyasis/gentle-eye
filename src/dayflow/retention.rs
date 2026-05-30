//! 3-tier retention: save → shrink → archive + disk-evict guard (Wave 6).
//!
//! Hot (raw chunks) → Warm (shrunk timelapse / change-threshold frames + OCR)
//! → Cold (timeline only, permanent). A disk-budget guard evicts oldest raw,
//! then oldest warm — never the timeline DB.
//!
//! TODO(Wave 6): `RetentionConfig` tier state machine + shrink + evict steps.
