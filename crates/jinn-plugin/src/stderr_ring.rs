//! Bounded ring buffer for captured guest stderr.
//!
//! Same shape as the MCP stderr buffer: newest content survives, memory is
//! bounded by a byte budget, multibyte characters are never split.

/// Default stderr tail budget (16&nbsp;KB).
const STDERR_MAX_BYTES: usize = 16 * 1024;

/// Bounded stderr ring buffer (most recent tail).
#[derive(Debug, Clone)]
pub struct StderrRing {
    content: String,
    max_bytes: usize,
}

impl Default for StderrRing {
    fn default() -> Self {
        Self::new()
    }
}

impl StderrRing {
    /// Creates a ring with the default 16&nbsp;KB budget.
    #[must_use]
    pub fn new() -> Self {
        Self::with_budget(STDERR_MAX_BYTES)
    }

    /// Creates a ring with a custom byte budget.
    #[must_use]
    pub fn with_budget(max_bytes: usize) -> Self {
        Self {
            content: String::new(),
            max_bytes,
        }
    }

    /// Appends one line and trims to the byte budget (keeping the tail).
    pub fn append_line(&mut self, line: &str) {
        if !self.content.is_empty() {
            self.content.push('\n');
        }
        self.content.push_str(line);
        self.trim_to_budget();
    }

    /// Drops leading content until the buffer fits the budget, advancing to
    /// the next UTF-8 char boundary so multibyte characters are never split.
    fn trim_to_budget(&mut self) {
        if self.content.len() <= self.max_bytes {
            return;
        }
        let cut = self.content.len().saturating_sub(self.max_bytes);
        let mut start = cut;
        while !self.content.is_char_boundary(start) {
            start += 1;
        }
        self.content.drain(0..start);
    }

    /// Returns the captured tail (may be empty).
    #[must_use]
    pub fn tail(&self) -> &str {
        &self.content
    }
}
