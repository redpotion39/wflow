//! Interface localisation (Thai / English).
//!
//! The source string is always English and doubles as the lookup key, so a
//! missing translation degrades gracefully to English instead of showing a
//! blank label or a raw key:
//!
//! ```ignore
//! ui.label(tr(lang, "Settings"));
//! ```
//!
//! Format strings are translated as templates and then formatted normally:
//!
//! ```ignore
//! ui.label(format!(tr(lang, "{} words"), count));
//! ```

/// Interface language. Independent of the dictation language.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Lang {
    /// Thai.
    #[default]
    Th,
    /// English.
    En,
}

impl Lang {
    /// Parses the value stored in `AppConfig::ui_language`.
    pub fn from_code(code: &str) -> Self {
        match code.trim().to_ascii_lowercase().as_str() {
            "en" | "english" => Lang::En,
            _ => Lang::Th,
        }
    }

    /// Value persisted in `AppConfig::ui_language`.
    pub fn code(self) -> &'static str {
        match self {
            Lang::Th => "th",
            Lang::En => "en",
        }
    }

    /// Name of the language in that language, for the picker.
    pub fn native_name(self) -> &'static str {
        match self {
            Lang::Th => "ไทย",
            Lang::En => "English",
        }
    }
}

/// Translates an English UI string into the interface language.
///
/// Unknown strings are returned unchanged, so new labels can be added without
/// touching this table first.
pub fn tr(lang: Lang, en: &'static str) -> &'static str {
    if lang == Lang::En {
        return en;
    }
    match en {
        // ---- Shell / navigation -------------------------------------------
        "AI Voice Studio" => "สตูดิโอเสียง AI",
        "Live Studio" => "สตูดิโอสด",
        "History" => "ประวัติ",
        "Dictionary" => "พจนานุกรม",
        "Updates" => "อัปเดต",
        "Settings" => "ตั้งค่า",
        "ENGINE" => "เอนจิน",
        "MIC" => "ไมค์",
        "HOTKEY" => "ปุ่มลัด",
        "VERSION" => "เวอร์ชัน",
        "Default Mic" => "ไมค์เริ่มต้น",
        "Exit" => "ปิดโปรแกรม",
        "Minimize ({})" => "ย่อหน้าต่าง ({})",
        "Minimized to Tray. Press {} to reopen." => "ย่อลงถาดระบบแล้ว กด {} เพื่อเปิดใหม่",
        "Language" => "ภาษา",

        // ---- Page titles and subtitles ------------------------------------
        "Live Speech Studio" => "สตูดิโอเสียงพูดสด",
        "Zero-latency real-time voice typing" => "พิมพ์ด้วยเสียงแบบเรียลไทม์ ไม่หน่วง",
        "Dictation History" => "ประวัติการพิมพ์ด้วยเสียง",
        "Search, copy, or re-paste past speech" => "ค้นหา คัดลอก หรือวางข้อความที่พูดไว้",
        "Custom Vocabulary & Snippets" => "คำศัพท์และข้อความย่อที่กำหนดเอง",
        "Domain terminology and text expansion shortcuts" => {
            "คำศัพท์เฉพาะทางและคำย่อขยายข้อความ"
        }
        "Software Updates" => "อัปเดตซอฟต์แวร์",
        "Check for a new version of Wflow and download it" => {
            "ตรวจหาเวอร์ชันใหม่ของ Wflow และดาวน์โหลด"
        }
        "Settings & Audio Hardware" => "ตั้งค่าและอุปกรณ์เสียง",
        "Configure microphones, AI models, and keyboard shortcuts" => {
            "ตั้งค่าไมโครโฟน โมเดล AI และปุ่มลัด"
        }

        // ---- Top bar status ------------------------------------------------
        "Dictate" => "เริ่มพูด",
        "Stop" => "หยุด",
        "Ready [{}]" => "พร้อม [{}]",
        "Recording..." => "กำลังบันทึก...",
        "AI Processing..." => "AI กำลังประมวลผล...",
        "Pasting Text..." => "กำลังวางข้อความ...",
        "Inserted!" => "แทรกข้อความแล้ว!",
        "Audio Error" => "เสียงผิดพลาด",
        "Focus: {}" => "โฟกัส: {}",
        "Desktop" => "เดสก์ท็อป",

        // ---- Update button --------------------------------------------------
        "Update · v{}" => "อัปเดต · v{}",
        "New update v{}" => "อัปเดตใหม่ v{}",
        "Checking..." => "กำลังตรวจสอบ...",
        "Check for updates" => "ตรวจสอบอัปเดต",
        "Download the new version" => "ดาวน์โหลดเวอร์ชันใหม่",

        // ---- Studio ---------------------------------------------------------
        "AUDIO CAPTURE & WAVEFORM" => "การบันทึกเสียงและรูปคลื่น",
        "Speaking" => "กำลังพูด",
        "Silent" => "เสียงเงียบ",
        "LIVE • {}" => "สด • {}",
        "START DICTATING (OR TAP {})" => "เริ่มพิมพ์ด้วยเสียง (หรือกด {})",
        "START DICTATING (OR HOLD {})" => "เริ่มพิมพ์ด้วยเสียง (หรือกดค้าง {})",
        "CLICK TO COMMIT & INSERT" => "คลิกเพื่อยืนยันและแทรกข้อความ",
        "AI TRANSCRIBING..." => "AI กำลังถอดเสียง...",
        "Tap {} once to start dictating hands-free, tap again to commit and insert text." => {
            "กด {} หนึ่งครั้งเพื่อเริ่มพูดแบบไม่ต้องกดค้าง กดอีกครั้งเพื่อยืนยันและแทรกข้อความ"
        }
        "Hold {} while speaking to dictate directly into any text box." => {
            "กด {} ค้างไว้ขณะพูด เพื่อพิมพ์ข้อความลงช่องข้อความใดก็ได้"
        }
        "TARGET APPLICATION CONTEXT" => "แอปเป้าหมาย",
        "Desktop Cursor" => "เคอร์เซอร์บนเดสก์ท็อป",
        "Before Cursor: \"{}\"" => "ข้อความก่อนเคอร์เซอร์: \"{}\"",
        "AI SPEECH ENGINE" => "เอนจินเสียง AI",
        "Switch Model" => "เปลี่ยนโมเดล",
        "Model: {}" => "โมเดล: {}",
        "LATEST TRANSCRIPTION" => "ข้อความล่าสุด",
        "No recent transcript." => "ยังไม่มีข้อความล่าสุด",
        "Press {} and speak clearly into your mic." => "กด {} แล้วพูดใส่ไมโครโฟนให้ชัดเจน",
        "{} words • {} chars" => "{} คำ • {} ตัวอักษร",
        "Re-paste" => "วางอีกครั้ง",
        "Copy" => "คัดลอก",
        "Clear" => "ล้าง",
        "RECORDED" => "จำนวนที่บันทึก",
        "{} Entries" => "{} รายการ",
        "LATENCY" => "ความหน่วง",
        "STORAGE" => "พื้นที่เก็บ",
        "SQLite Local" => "SQLite ในเครื่อง",
        "RECENT ACTIVITY" => "กิจกรรมล่าสุด",
        "View All →" => "ดูทั้งหมด →",
        "No recent transcriptions yet. Dictate anywhere to see activity." => {
            "ยังไม่มีรายการที่พูดไว้ เริ่มพูดที่ไหนก็ได้เพื่อดูกิจกรรม"
        }
        "Copied to clipboard!" => "คัดลอกไปยังคลิปบอร์ดแล้ว!",
        "Copied transcript to clipboard!" => "คัดลอกข้อความไปยังคลิปบอร์ดแล้ว!",
        "Transcript cleared" => "ล้างข้อความแล้ว",
        "Re-pasted transcript to active cursor!" => "วางข้อความซ้ำที่เคอร์เซอร์แล้ว!",

        // ---- Settings sections ----------------------------------------------
        "Shortcuts" => "ปุ่มลัด",
        "Audio" => "เสียง",
        "AI & Models" => "AI และโมเดล",
        "Voice & Text" => "เสียงและข้อความ",
        "Privacy" => "ความเป็นส่วนตัว",
        "Save All Preferences" => "บันทึกการตั้งค่าทั้งหมด",

        // ---- HUD -------------------------------------------------------------
        "Listening..." => "กำลังฟัง...",
        "Processing..." => "กำลังประมวลผล...",
        "Text inserted" => "แทรกข้อความแล้ว",
        "Something went wrong" => "เกิดข้อผิดพลาด",
        "Ready" => "พร้อมใช้งาน",
        "Stop recording" => "หยุดบันทึก",

        // ---- Exit dialog -------------------------------------------------------
        "Confirm Exit" => "ยืนยันการปิดโปรแกรม",
        "Do you really want to quit Wflow?" => "คุณต้องการปิดโปรแกรม Wflow ใช่หรือไม่?",
        "Minimize to Tray" => "ย่อลงถาดระบบ",
        "Cancel" => "ยกเลิก",
        "If you minimize to the tray, Wflow keeps listening for your dictation hotkey in the background." => {
            "หากย่อลงถาดระบบ Wflow จะยังคงรอรับคำสั่งเสียงผ่านปุ่มลัดอยู่เบื้องหลัง"
        }

        " hours" => "ชั่วโมง",
        " sec" => "วินาที",
        "(0 = never stop automatically)" => "(0 = ไม่หยุดอัตโนมัติ)",
        "(leave blank = detect from the text automatically)" => "(เว้นว่าง = ตรวจตามภาษาของข้อความอัตโนมัติ)",
        "(no further details)" => "(ไม่มีรายละเอียดเพิ่มเติม)",
        "(press Save All Preferences to apply)" => "(กดบันทึกการตั้งค่าเพื่อใช้งาน)",
        "AI POST-PROCESSING & SMART FORMATTING" => "การปรับแต่งข้อความด้วย AI",
        "API Endpoint URL:" => "URL ของ API:",
        "Add Entry" => "เพิ่มรายการ",
        "Add custom words or text expansion snippets above!" => "เพิ่มคำศัพท์หรือข้อความย่อได้ด้านบน!",
        "Added '{}' to dictionary" => "เพิ่ม '{}' ลงพจนานุกรมแล้ว",
        "All Entries ({})" => "ทั้งหมด ({})",
        "Audio input devices refreshed" => "รีเฟรชรายการไมโครโฟนแล้ว",
        "Auto-detected from local Wispr Flow session if installed" => "ตรวจพบอัตโนมัติจากเซสชัน Wispr Flow ในเครื่อง (ถ้ามี)",
        "Auto-paste transcribed text directly into active cursor position" => "วางข้อความที่ถอดเสียงลงตำแหน่งเคอร์เซอร์อัตโนมัติ",
        "Auto-start on system startup disabled" => "ปิดการเริ่มอัตโนมัติแล้ว",
        "Auto-start on system startup enabled" => "เปิดการเริ่มอัตโนมัติแล้ว",
        "Automatic spell check" => "ตรวจสะกดอัตโนมัติ",
        "Automatically cleans speech disfluencies, filler words (um, uh, อ่า, แบบว่า), fixes punctuation, and adapts writing tone." => "ลบคำพูดติดขัดและคำเกิน (อ่า, แบบว่า) แก้เครื่องหมายวรรคตอน และปรับโทนการเขียนให้เหมาะสมโดยอัตโนมัติ",
        "Baseten Gateway API Key (optional, stored securely):" => "API Key ของ Baseten (ไม่บังคับ เก็บอย่างปลอดภัย):",
        "Bearer Auth Token (Optional / Session Token):" => "โทเค็นยืนยันตัวตน (ไม่บังคับ / โทเค็นเซสชัน):",
        "Bullet Points (Summary in structured markdown bullets)" => "หัวข้อย่อย (สรุปเป็นรายการ markdown)",
        "CUSTOM VOCABULARY & TEXT SNIPPET EXPANSIONS" => "คำศัพท์และข้อความย่อที่กำหนดเอง",
        "Casual & Friendly (Chat and messaging tone)" => "เป็นกันเอง (โทนแชทและข้อความ)",
        "Changes on this page apply after pressing Save All Preferences" => "การเปลี่ยนแปลงในหน้านี้จะมีผลหลังกดบันทึกการตั้งค่า",
        "Check every:" => "ตรวจสอบทุก:",
        "Check for new versions in the background" => "ตรวจหาเวอร์ชันใหม่ให้อัตโนมัติระหว่างใช้งาน",
        "Install new versions automatically" => "ติดตั้งเวอร์ชันใหม่ให้อัตโนมัติ",
        "Downloads the release for this platform, verifies its checksum, then restarts Wflow. It waits until you are not dictating." => {
            "ดาวน์โหลดเวอร์ชันสำหรับระบบของคุณ ตรวจสอบ checksum แล้วเริ่ม Wflow ใหม่อัตโนมัติ โดยจะรอจนคุณหยุดพูดก่อน"
        }
        "Check now" => "ตรวจสอบเดี๋ยวนี้",
        "Checked against GitHub Releases" => "ตรวจสอบผ่าน GitHub Releases",
        "Checked automatically at startup and at the interval set in Settings" => "ตรวจอัตโนมัติเมื่อเปิดโปรแกรมและตามช่วงเวลาที่ตั้งไว้ในหน้าตั้งค่า",
        "Checking for updates..." => "กำลังตรวจสอบอัปเดต...",
        "Choose the language of the Wflow interface. The language used for speech transcription is configured separately under Audio." => "เลือกภาษาที่ใช้แสดงผลหน้าจอ Wflow ส่วนภาษาที่ใช้ถอดเสียงพูดตั้งแยกได้ในหมวดเสียง",
        "Clear all dictation history" => "ล้างประวัติการพิมพ์ด้วยเสียงทั้งหมด",
        "Click again to clear all history" => "กดอีกครั้งเพื่อล้างประวัติทั้งหมด",
        "Click the button again to confirm clearing history" => "กดปุ่มอีกครั้งเพื่อยืนยันการล้างประวัติ",
        "Click to check for or download an update" => "คลิกเพื่อตรวจสอบหรือดาวน์โหลดอัปเดต",
        "Code & CLI Commands (Converts speech to code / terminal syntax)" => "โค้ดและคำสั่ง CLI (แปลงคำพูดเป็นโค้ด/คำสั่ง)",
        "Current version: v{}" => "เวอร์ชันปัจจุบัน: v{}",
        "Data is stored in: {}" => "ข้อมูลถูกเก็บไว้ที่: {}",
        "Default System Microphone" => "ไมโครโฟนเริ่มต้นของระบบ",
        "Delete" => "ลบ",
        "Details" => "รายละเอียด",
        "Detects invented text the model tends to add on its own, such as \"Thank you.\" or \"(upbeat music)\", and long repeated words, then drops it instead of typing it." => "ตรวจจับข้อความที่โมเดลมักแต่งขึ้นเอง เช่น \"Thank you.\" หรือ \"(upbeat music)\" รวมถึงคำที่ซ้ำยาว ๆ แล้วตัดทิ้งแทนที่จะพิมพ์ลงไป",
        "Dictated Words" => "จำนวนคำที่พูด",
        "Dictation Trigger Mode:" => "รูปแบบการเริ่มพูด:",
        "Dictation hotkey changed to: {}" => "เปลี่ยนปุ่มลัดเริ่มพูดเป็น: {}",
        "Displays an on-screen pill indicator with animated audio waveform bars when dictating without stealing focus." => "แสดงแถบสถานะลอยพร้อมรูปคลื่นเสียงขณะพูด โดยไม่แย่งโฟกัสหน้าต่าง",
        "Do not notify about this version again" => "ไม่ต้องแจ้งเตือนเวอร์ชันนี้อีก",
        "Download" => "ดาวน์โหลด",
        "Download v{}" => "ดาวน์โหลด v{}",
        "Download manually" => "ดาวน์โหลดเอง",
        "Download the installer and install it yourself" => "ดาวน์โหลดตัวติดตั้งแล้วติดตั้งด้วยตัวเอง",
        "Downloading update..." => "กำลังดาวน์โหลดอัปเดต...",
        "Installing update..." => "กำลังติดตั้งอัปเดต...",
        "Installing v{}..." => "กำลังติดตั้ง v{}...",
        "Install v{}" => "ติดตั้ง v{}",
        "Restarting Wflow..." => "กำลังเริ่ม Wflow ใหม่...",
        "Verifying checksum..." => "กำลังตรวจสอบ checksum...",
        "Working..." => "กำลังทำงาน...",
        "ElevenLabs API Key (get from elevenlabs.io):" => "API Key ของ ElevenLabs (รับได้จาก elevenlabs.io):",
        "ElevenLabs STT Model:" => "โมเดล ElevenLabs STT:",
        "ElevenLabs Scribe STT provides unmatched speech recognition accuracy across languages." => "ElevenLabs Scribe ให้ความแม่นยำในการถอดเสียงหลายภาษาที่เหนือกว่า",
        "Enable Instant AI Text Formatting & Cleanup (Groq Llama 3.1 8B ~150ms)" => "เปิดการจัดข้อความด้วย AI ทันที (Groq Llama 3.1 8B ~150ms)",
        "Enable Synthesized Audio Earcons (Start/Finish/Error sound effects)" => "เปิดเสียงแจ้งเตือน (เริ่ม/สิ้นสุด/ผิดพลาด)",
        "Entry removed" => "ลบรายการแล้ว",
        "For example \"ทดสอบAPI\" → \"ทดสอบ API\". Also fixes common misspellings in each language." => "ตัวอย่าง \"ทดสอบAPI\" → \"ทดสอบ API\" และแก้คำสะกดผิดที่พบบ่อยในแต่ละภาษา",
        "GLOBAL KEYBOARD SHORTCUTS" => "ปุ่มลัดทั้งระบบ",
        "Groq Cloud API Key (get from console.groq.com):" => "API Key ของ Groq (รับได้จาก console.groq.com):",
        "Groq Whisper Model:" => "โมเดล Groq Whisper:",
        "Groq executes Whisper on LPUs with near-zero latency (~200ms per utterance)." => "Groq รัน Whisper บน LPU ด้วยความหน่วงใกล้ศูนย์ (~200ms ต่อหนึ่งประโยค)",
        "Hallucination guard" => "กันข้อความที่โมเดลแต่งขึ้นเอง",
        "History records refreshed!" => "รีเฟรชประวัติแล้ว!",
        "Hold down your hotkey while speaking. Releasing the key immediately commits and types the text." => "กดปุ่มลัดค้างไว้ขณะพูด เมื่อปล่อยปุ่มระบบจะยืนยันและพิมพ์ข้อความทันที",
        "Hold key to talk (Push-to-Talk)" => "กดค้างเพื่อพูด",
        "Hold this key or combination to record voice, release to insert text." => "กดปุ่มนี้ค้างไว้เพื่อบันทึกเสียง แล้วปล่อยเพื่อแทรกข้อความ",
        "INTERFACE LANGUAGE" => "ภาษาของหน้าจอ",
        "If unchecked, a confirmation dialog appears when you press [X]." => "หากไม่เลือก จะมีหน้าต่างยืนยันเมื่อกดปุ่ม [X]",
        "Include pre-release versions" => "รวมเวอร์ชันทดลอง (pre-release)",
        "Input Microphone:" => "ไมโครโฟนที่ใช้:",
        "Key recording cancelled" => "ยกเลิกการบันทึกปุ่มแล้ว",
        "LANGUAGE & FLOATING OVERLAY HUD" => "ภาษาและแถบสถานะลอย",
        "Languages to spell check:" => "ภาษาที่ต้องการตรวจสะกด:",
        "Last checked: {}" => "ตรวจสอบล่าสุด: {}",
        "Launches Wflow automatically when your system boots." => "เปิด Wflow อัตโนมัติเมื่อระบบเริ่มทำงาน",
        "Less sensitive (keeps quiet speech)" => "ไวน้อย (เก็บเสียงพูดเบา ๆ)",
        "Listening for shortcut combination... (Esc to cancel)" => "กำลังฟังชุดปุ่ม... (กด Esc เพื่อยกเลิก)",
        "MICROPHONE HARDWARE & AUDIO SOUND EFFECTS" => "ไมโครโฟนและเสียงแจ้งเตือน",
        "Maximum recording length (seconds):" => "ความยาวบันทึกสูงสุด (วินาที):",
        "Minimize to tray on close" => "ย่อลงถาดระบบเมื่อกดปิด",
        "Mixed-language spacing" => "เติมช่องว่างระหว่างภาษา",
        "Model ID: {}" => "รหัสโมเดล: {}",
        "Native gRPC Protobuf streaming" => "สตรีมผ่าน gRPC แบบเนทีฟ",
        "New version available: v{} (current v{})" => "มีเวอร์ชันใหม่: v{} (ปัจจุบัน v{})",
        "New version v{} available" => "มีเวอร์ชันใหม่ v{}",
        "No dictionary entries found." => "ยังไม่มีรายการในพจนานุกรม",
        "No matching dictation records found." => "ไม่พบรายการที่ตรงกัน",
        "Normal" => "ปกติ",
        "Open Log Folder" => "เปิดโฟลเดอร์ Log",
        "Open Releases page" => "เปิดหน้า Releases",
        "Open the GitHub Releases page in your browser" => "เปิดหน้า GitHub Releases ในเบราว์เซอร์",
        "Opening the download page for v{}" => "กำลังเปิดหน้าดาวน์โหลดสำหรับ v{}",
        "PRIVACY & LOCAL DATA" => "ความเป็นส่วนตัวและข้อมูลในเครื่อง",
        "Paste" => "วาง",
        "Pasted into active window!" => "วางลงในหน้าต่างที่ใช้งานอยู่แล้ว!",
        "Preset: {}" => "ค่าที่ตั้งไว้: {}",
        "Press any combination (e.g. Alt+Space, Ctrl+Space, F9) to set" => "กดชุดปุ่มที่ต้องการ (เช่น Alt+Space, Ctrl+Space, F9)",
        "Press any combination (e.g. Ctrl+Space, Alt+Space, Ctrl+Shift+D) to set" => "กดชุดปุ่มที่ต้องการ (เช่น Ctrl+Space, Alt+Space, Ctrl+Shift+D)",
        "Press any key or combination to set" => "กดปุ่มหรือชุดปุ่มที่ต้องการ",
        "Press this key or combination anywhere to toggle Wflow Studio between screen and system tray." => "กดปุ่มนี้ที่ไหนก็ได้เพื่อสลับหน้าต่าง Wflow ระหว่างหน้าจอกับถาดระบบ",
        "Professional & Executive (Polite corporate emails & reports)" => "มืออาชีพ (อีเมลและรายงานธุรกิจที่สุภาพ)",
        "Push-To-Talk Dictation Hotkey:" => "ปุ่มลัดกดค้างเพื่อพูด:",
        "Push-to-Talk (Hold Key)" => "กดค้างเพื่อพูด",
        "Record deleted" => "ลบรายการแล้ว",
        "Recording stops automatically at this limit to protect memory." => "ระบบจะหยุดบันทึกอัตโนมัติเมื่อถึงขีดจำกัดนี้ เพื่อป้องกันหน่วยความจำเต็ม",
        "Refresh" => "รีเฟรช",
        "Refresh Devices" => "รีเฟรชอุปกรณ์",
        "SNIPPET" => "ข้อความย่อ",
        "SOFTWARE UPDATES & DIAGNOSTICS" => "อัปเดตซอฟต์แวร์และการวินิจฉัย",
        "SPEECH-TO-TEXT (STT) AI SERVICE PROVIDER" => "ผู้ให้บริการถอดเสียงเป็นข้อความ (STT)",
        "STT request timeout (seconds):" => "หมดเวลารอคำขอ STT (วินาที):",
        "Search transcripts by keywords or application name..." => "ค้นหาข้อความหรือชื่อแอป...",
        "Settings & STT configurations saved successfully!" => "บันทึกการตั้งค่าและการเชื่อมต่อ STT เรียบร้อยแล้ว!",
        "Share surrounding text context with the STT provider" => "ส่งข้อความรอบเคอร์เซอร์ให้ผู้ให้บริการ STT",
        "Show floating recording symbol & live waveform HUD (overlay)" => "แสดงแถบสถานะลอยพร้อมรูปคลื่นเสียง",
        "Silence filter (noise gate)" => "กรองเสียงเงียบ (noise gate)",
        "Skip this version" => "ข้ามเวอร์ชันนี้",
        "Smart Cleanup (Removes stutters, filler words, fixes punctuation)" => "จัดระเบียบอัจฉริยะ (ลบคำติดขัด คำเกิน แก้เครื่องหมายวรรคตอน)",
        "Smart Cleanup (Removes stutters, filler words, fixes punctuation) [Recommended]" => "จัดระเบียบอัจฉริยะ (ลบคำติดขัด คำเกิน แก้เครื่องหมายวรรคตอน) [แนะนำ]",
        "Snippet Replacement (Optional):" => "ข้อความแทนที่ (ไม่บังคับ):",
        "Soft pure-Rust synthesized harmonic chimes provide immediate tactile feedback when dictation starts and finishes." => "เสียงประสานสังเคราะห์แบบบริสุทธิ์ใน Rust ให้ผลตอบรับทันทีเมื่อเริ่มและสิ้นสุดการพูด",
        "Speech sensitivity:" => "ความไวต่อเสียงพูด:",
        "Start Wflow automatically on system startup (Run on Startup)" => "เริ่ม Wflow อัตโนมัติเมื่อเปิดเครื่อง",
        "Stop automatically after silence:" => "หยุดอัตโนมัติเมื่อเงียบนาน:",
        "Store textbox contents in the local history" => "เก็บเนื้อหาช่องข้อความไว้ในประวัติในเครื่อง",
        "Tap once to start / tap to stop (Hands-Free)" => "กดครั้งเดียวเริ่ม / กดอีกครั้งหยุด (ไม่ต้องกดค้าง)",
        "Tap to Toggle (Hands-Free)" => "กดเพื่อเปิด/ปิด (ไม่ต้องกดค้าง)",
        "Tap your hotkey once to start dictating freely. Tap again to stop and automatically paste into your active window." => "กดปุ่มลัดหนึ่งครั้งเพื่อเริ่มพูดได้อย่างอิสระ กดอีกครั้งเพื่อหยุดและวางข้อความลงในหน้าต่างที่ใช้งานอยู่",
        "Teach Wflow custom terminology, names, or define instant trigger snippets. When you speak a trigger phrase, it will automatically be expanded into your configured replacement text." => "สอนคำศัพท์ ชื่อเฉพาะ หรือสร้างข้อความย่อให้ Wflow เมื่อคุณพูดวลีที่กำหนด ระบบจะแทนที่ด้วยข้อความที่ตั้งไว้ทันที",
        "Test Chime" => "ทดสอบเสียง",
        "Test STT Provider" => "ทดสอบการเชื่อมต่อ STT",
        "Testing the STT connection..." => "กำลังทดสอบการเชื่อมต่อ STT...",
        "The interface language only affects labels and messages. Your dictation hotkeys, history and vocabulary are unchanged." => "ภาษาของหน้าจอมีผลเฉพาะข้อความและป้ายกำกับ ปุ่มลัด ประวัติ และคำศัพท์ของคุณจะไม่เปลี่ยนแปลง",
        "Toggle Window Hotkey:" => "ปุ่มลัดเปิด/ปิดหน้าต่าง:",
        "Total Transcriptions" => "จำนวนการถอดเสียง",
        "Total Voice Time" => "เวลาพูดรวม",
        "Transcription Language:" => "ภาษาที่ใช้ถอดเสียง:",
        "Trims silence and background noise before audio reaches the STT model, reducing errors and saving bandwidth." => "ตัดเสียงเงียบและเสียงรบกวนก่อนส่งให้โมเดล STT ช่วยลดการถอดผิดและประหยัดแบนด์วิดท์",
        "Turn off for maximum privacy: before/after text and selected text are never uploaded (recommended for sensitive text)." => "ปิดเพื่อความเป็นส่วนตัวสูงสุด: ข้อความก่อน/หลังเคอร์เซอร์และข้อความที่เลือกจะไม่ถูกอัปโหลด (แนะนำเมื่อพิมพ์ข้อความลับ)",
        "Update check failed: {}" => "ตรวจสอบอัปเดตไม่สำเร็จ: {}",
        "Use a language model for spelling (requires a Groq API key)" => "ใช้โมเดลภาษาช่วยตรวจสะกด (ต้องมี Groq API key)",
        "VOCAB" => "คำศัพท์",
        "VOICE FILTER & TEXT QUALITY" => "ตัวกรองเสียงและคุณภาพข้อความ",
        "Version skipped" => "ข้ามเวอร์ชันนี้แล้ว",
        "Very sensitive" => "ไวมาก",
        "View Release Notes" => "ดู Release Notes",
        "View Release Notes on GitHub" => "ดู Release Notes บน GitHub",
        "Wflow Software Update" => "อัปเดตซอฟต์แวร์ Wflow",
        "Window toggle hotkey changed to: {}" => "เปลี่ยนปุ่มลัดเปิด/ปิดหน้าต่างเป็น: {}",
        "Word or Trigger Phrase:" => "คำหรือวลีที่ต้องการ:",
        "Writing Style Tone:" => "โทนการเขียน:",
        "You are on the latest version" => "เป็นเวอร์ชันล่าสุดแล้ว",
        "You have unsaved changes" => "มีการเปลี่ยนแปลงที่ยังไม่ได้บันทึก",
        "e.g. Wflow, myemail, BRB, หมอพร้อม" => "เช่น Wflow, myemail, BRB, หมอพร้อม",
        "e.g. contact@wflow.ai (leave blank for vocabulary only)" => "เช่น contact@wflow.ai (เว้นว่างหากใช้เป็นคำศัพท์เท่านั้น)",
        "e.g. th,en" => "เช่น th,en",
        "never" => "ยังไม่เคยตรวจสอบ",
        "{} seconds" => "{} วินาที",
        "{} recorded" => "{} รายการ",
        "{} words" => "{} คำ",
        "🔍 Filter dictionary..." => "🔍 ค้นหาคำศัพท์...",

        "Minimized to the tray. Press {} or click the tray icon to reopen." => "ย่อลงถาดระบบแล้ว กด {} หรือคลิกไอคอนถาดระบบเพื่อเปิดใหม่",

        _ => en,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn english_is_returned_verbatim() {
        assert_eq!(tr(Lang::En, "Settings"), "Settings");
    }

    #[test]
    fn unknown_keys_fall_back_to_english() {
        assert_eq!(tr(Lang::Th, "Not translated yet"), "Not translated yet");
    }

    #[test]
    fn thai_translation_is_used() {
        assert_eq!(tr(Lang::Th, "Settings"), "ตั้งค่า");
        assert_eq!(tr(Lang::Th, "Listening..."), "กำลังฟัง...");
    }

    #[test]
    fn language_codes_round_trip() {
        assert_eq!(Lang::from_code("en"), Lang::En);
        assert_eq!(Lang::from_code("TH"), Lang::Th);
        assert_eq!(Lang::from_code("fr"), Lang::Th);
        assert_eq!(Lang::En.code(), "en");
    }
}
