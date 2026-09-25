//! Agent skills - the kernel UI-bound surface.
//!
//! Portable values and crossing contracts live in `jinn-skills-msg`; the picker
//! itself - entry construction, preview cache, and reload - now lives in
//! `jinn-skills`. Nothing is left here, so this module is a placeholder kept to
//! hold the kernel's `feat::skills` path until that path is retired.
//!
//! This module deliberately re-exports nothing from `jinn-skills`: every real
//! consumer imports that crate directly, and a re-export would make the kernel a
//! production dependent of the slice - which would, in turn, forbid the slice
//! from depending on the kernel and so freeze the skill picker in this crate
//! forever.
