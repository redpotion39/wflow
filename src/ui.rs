/// The state of the dictation pipeline, shared by the engine and the GUI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DictationStatus {
    Idle,
    Listening,
    Transcribing,
    Pasting,
    Done,
    Error,
}

impl DictationStatus {
    /// True while the pipeline is actively working, i.e. the HUD should be
    /// visible and the UI should keep repainting.
    pub fn is_busy(&self) -> bool {
        !matches!(self, DictationStatus::Idle)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idle_is_the_only_non_busy_state() {
        assert!(!DictationStatus::Idle.is_busy());
        assert!(DictationStatus::Listening.is_busy());
        assert!(DictationStatus::Transcribing.is_busy());
        assert!(DictationStatus::Pasting.is_busy());
        assert!(DictationStatus::Done.is_busy());
        assert!(DictationStatus::Error.is_busy());
    }
}
