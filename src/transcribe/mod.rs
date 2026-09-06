//! Screen-text transcription primitives.
//!
//! Five deterministic pieces an agent chains into a transcript:
//!
//! - [`frames`] — the frames of a recording, each with a **sharpness** score.
//! - [`quality`] — the **information content** of a piece of text.
//! - [`reader`] — per-model adapters that own a prompt and normalise a response.
//! - [`stack`] — the best single image from N frames of ONE screen, with scores
//!   (feature-gated on `tracking`; the default build states that, never guesses).
//! - [`locate`] — where the screen is in a frame, which of its corners are
//!   clipped, and how it was found (feature-gated the same way).
//!
//! # What lives here, and what deliberately does not
//!
//! Each primitive answers ONE question and decides nothing. Every threshold —
//! how sharp is sharp enough, how much repetition is too much, how similar two
//! lines must be to count as the same line — belongs to the CALLER.
//!
//! That is not squeamishness. It was measured: the same 15-second recording
//! keeps 285, 138 or 2 frames depending on one deduplication threshold, because
//! scrolling text genuinely changes every frame while slides do not. A judgement
//! that must vary with the content cannot be a constant in a binary
//! (`specs/015-screen-transcription/research.md`, M1 and D015-7).
//!
//! The merge primitive is deliberately absent from this module: it already
//! exists as [`crate::dayflow::perception::merge_scroll`] and is made reachable
//! there rather than reimplemented here. A second copy beside an unused first is
//! the defect this feature exists to close (D015-8).

pub mod frames;
pub mod locate;
pub mod quality;
pub mod reader;
pub mod stack;
