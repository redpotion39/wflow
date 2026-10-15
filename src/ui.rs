#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DictationStatus {
    Idle,
    Listening,
    Transcribing,
    Pasting,
    Done,
    Error,
}

#[allow(dead_code)]
pub struct StatusOverlay;

impl StatusOverlay {
    #[allow(dead_code)]
    pub fn update(status: DictationStatus, amplitude: f32) {
        match status {
            DictationStatus::Idle => {
                // Console status display
            }
            DictationStatus::Listening => {
                let bars = (amplitude * 50.0).clamp(0.0, 20.0) as usize;
                let wave: String = "█".repeat(bars);
                print!("\r[🎙️  LISTENING] {:<20} (Release F8 to finish)", wave);
                let _ = std::io::Write::flush(&mut std::io::stdout());
            }
            DictationStatus::Transcribing => {
                print!("\r[⏳ TRANSCRIBING & FORMATTING...]                           \r");
                let _ = std::io::Write::flush(&mut std::io::stdout());
            }
            DictationStatus::Pasting => {
                print!("\r[✍️  PASTING TEXT...]                                          \r");
                let _ = std::io::Write::flush(&mut std::io::stdout());
            }
            DictationStatus::Done => {
                println!("\r[✅ DONE & INSERTED]                                         ");
            }
            DictationStatus::Error => {
                println!("\r[⚠️  ERROR ENCOUNTERED]                                      ");
            }
        }
    }
}
