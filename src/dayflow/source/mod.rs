//! Capture sources — where a Dayflow sample comes from.
//!
//! A source is either an **input taken** (a stream, a capture card, a camera —
//! content that may never be rendered on this machine's screen) or a **display
//! consumed** (a screen, a named window, a target region). The two are co-equal
//! kinds, not a primary and a special case.
//!
//! The trait and its implementations land in T003-T005; this module exists first
//! so the loop and the source kinds can be developed against a declared path.
