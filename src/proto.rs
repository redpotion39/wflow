// Reconstructed flow_api.v1 Protocol Buffer definitions for Wispr Flow

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, ::prost::Enumeration)]
#[repr(i32)]
pub enum Platform {
    Unspecified = 0,
    Macos = 1,
    Win32 = 2,
    Ios = 3,
    Android = 4,
    Web = 5,
    Api = 6,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, ::prost::Enumeration)]
#[repr(i32)]
pub enum AppType {
    Unspecified = 0,
    Other = 1,
    Browser = 2,
    PersonalMessaging = 3,
    WorkMessaging = 4,
    Email = 5,
    Chatbot = 6,
    Developer = 7,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, ::prost::Enumeration)]
#[repr(i32)]
pub enum AudioEncoding {
    Unspecified = 0,
    Wav = 1,
    Opus = 2,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, ::prost::Enumeration)]
#[repr(i32)]
pub enum Environment {
    Unspecified = 0,
    Production = 1,
    Staging = 2,
    Development = 3,
    Test = 4,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, ::prost::Enumeration)]
#[repr(i32)]
pub enum WritingStyle {
    Unspecified = 0,
    Formal = 1,
    Casual = 2,
    VeryCasual = 3,
    Excited = 4,
    Genz = 5,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, ::prost::Enumeration)]
#[repr(i32)]
pub enum EditingStrength {
    Unspecified = 0,
    Verbatim = 1,
    Light = 2,
    Medium = 3,
    Heavy = 4,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, ::prost::Enumeration)]
#[repr(i32)]
pub enum FlowSignaturePrefix {
    Unspecified = 0,
    Spoken = 1,
    Written = 2,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, ::prost::Enumeration)]
#[repr(i32)]
pub enum Confidence {
    Unspecified = 0,
    None = 1,
    Low = 2,
    Medium = 3,
    High = 4,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, ::prost::Enumeration)]
#[repr(i32)]
pub enum Origin {
    Unspecified = 0,
    Mixture = 1,
    Whisper = 2,
    Scribe = 3,
    Voxtral = 4,
    Qwen = 5,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, ::prost::Enumeration)]
#[repr(i32)]
pub enum Commit {
    Unspecified = 0,
    True = 1,
    False = 2,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, ::prost::Enumeration)]
#[repr(i32)]
pub enum SignatureStatus {
    Unspecified = 0,
    Eligible = 1,
    Added = 2,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, ::prost::Enumeration)]
#[repr(i32)]
pub enum ResultStatus {
    Unspecified = 0,
    Formatted = 1,
    Error = 2,
    RawTranscript = 3,
}

#[derive(Clone, PartialEq, ::prost::Message)]
pub struct Version {
    #[prost(uint32, tag = "1")]
    pub major: u32,
    #[prost(uint32, tag = "2")]
    pub minor: u32,
    #[prost(uint32, tag = "3")]
    pub patch: u32,
}

#[derive(Clone, PartialEq, ::prost::Message)]
pub struct Client {
    #[prost(string, tag = "1")]
    pub name: String,
    #[prost(enumeration = "Platform", tag = "2")]
    pub platform: i32,
    #[prost(message, optional, tag = "3")]
    pub version: Option<Version>,
}

#[derive(Clone, PartialEq, ::prost::Message)]
pub struct Metadata {
    #[prost(string, tag = "1")]
    pub user_id: String,
    #[prost(string, tag = "2")]
    pub session_id: String,
    #[prost(string, tag = "3")]
    pub request_id: String,
    #[prost(enumeration = "AudioEncoding", tag = "4")]
    pub audio_encoding: i32,
    #[prost(enumeration = "Environment", tag = "5")]
    pub environment: i32,
    #[prost(message, optional, tag = "6")]
    pub client: Option<Client>,
    #[prost(bool, tag = "7")]
    pub debug: bool,
    #[prost(string, tag = "8")]
    pub referral_code: String,
    #[prost(bool, tag = "9")]
    pub always_wait_scribe: bool,
}

#[derive(Clone, PartialEq, ::prost::Message)]
pub struct User {
    #[prost(string, tag = "1")]
    pub first_name: String,
    #[prost(string, tag = "2")]
    pub last_name: String,
    #[prost(string, tag = "3")]
    pub email: String,
}

#[derive(Clone, PartialEq, ::prost::Message)]
pub struct StaticVocabulary {
    #[prost(string, repeated, tag = "1")]
    pub dictionary_personal: Vec<String>,
    #[prost(string, repeated, tag = "2")]
    pub dictionary_team: Vec<String>,
    #[prost(string, repeated, tag = "3")]
    pub dictionary_personal_starred: Vec<String>,
    #[prost(string, repeated, tag = "4")]
    pub dictionary_team_starred: Vec<String>,
}

#[derive(Clone, PartialEq, ::prost::Message)]
pub struct Replacements {
    #[prost(map = "string, string", tag = "1")]
    pub replacements_personal: ::std::collections::HashMap<String, String>,
    #[prost(map = "string, string", tag = "2")]
    pub replacements_team: ::std::collections::HashMap<String, String>,
    #[prost(map = "string, string", tag = "3")]
    pub snippets_personal: ::std::collections::HashMap<String, String>,
    #[prost(map = "string, string", tag = "4")]
    pub snippets_team: ::std::collections::HashMap<String, String>,
}

#[derive(Clone, PartialEq, ::prost::Message)]
pub struct GeneralWritingStyle {
    #[prost(enumeration = "WritingStyle", tag = "1")]
    pub other: i32,
    #[prost(enumeration = "WritingStyle", tag = "2")]
    pub personal: i32,
    #[prost(enumeration = "WritingStyle", tag = "3")]
    pub work: i32,
}

#[derive(Clone, PartialEq, ::prost::Message)]
pub struct TaggingStyle {
    #[prost(bool, optional, tag = "1")]
    pub slack: Option<bool>,
}

#[derive(Clone, PartialEq, ::prost::Message)]
pub struct SignatureStyle {
    #[prost(enumeration = "FlowSignaturePrefix", tag = "1")]
    pub flow_signature_prefix: i32,
    #[prost(bool, optional, tag = "2")]
    pub flow_signature_hyperlink: Option<bool>,
    #[prost(bool, optional, tag = "3")]
    pub sign_emails: Option<bool>,
}

#[derive(Clone, PartialEq, ::prost::Message)]
pub struct StyleConfig {
    #[prost(message, optional, tag = "1")]
    pub general_style: Option<GeneralWritingStyle>,
    #[prost(enumeration = "WritingStyle", tag = "2")]
    pub override_style: i32,
    #[prost(message, optional, tag = "3")]
    pub tagging: Option<TaggingStyle>,
    #[prost(message, optional, tag = "4")]
    pub signature: Option<SignatureStyle>,
}

#[derive(Clone, PartialEq, ::prost::Message)]
pub struct Preferences {
    #[prost(message, optional, tag = "1")]
    pub user: Option<User>,
    #[prost(int32, repeated, tag = "2")]
    pub language: Vec<i32>,
    #[prost(message, optional, tag = "3")]
    pub vocabulary: Option<StaticVocabulary>,
    #[prost(message, optional, tag = "4")]
    pub replacements: Option<Replacements>,
    #[prost(message, optional, tag = "5")]
    pub style_config: Option<StyleConfig>,
}

#[derive(Clone, PartialEq, ::prost::Message)]
pub struct App {
    #[prost(string, tag = "1")]
    pub name: String,
    #[prost(string, tag = "2")]
    pub bundle_id: String,
    #[prost(string, tag = "3")]
    pub url: String,
    #[prost(enumeration = "AppType", tag = "4")]
    pub r#type: i32,
}

#[derive(Clone, PartialEq, ::prost::Message)]
pub struct Textbox {
    #[prost(string, optional, tag = "1")]
    pub contents: Option<String>,
    #[prost(string, optional, tag = "2")]
    pub before_text: Option<String>,
    #[prost(string, optional, tag = "3")]
    pub selected_text: Option<String>,
    #[prost(string, optional, tag = "4")]
    pub after_text: Option<String>,
}

#[derive(Clone, PartialEq, ::prost::Message)]
pub struct DynamicVocabulary {
    #[prost(string, repeated, tag = "1")]
    pub screen_content_ax: Vec<String>,
    #[prost(string, repeated, tag = "2")]
    pub screen_content_ocr: Vec<String>,
    #[prost(string, repeated, tag = "3")]
    pub variable_names: Vec<String>,
    #[prost(string, repeated, tag = "4")]
    pub file_names: Vec<String>,
}

#[derive(Clone, PartialEq, ::prost::Message)]
pub struct Context {
    #[prost(message, optional, tag = "1")]
    pub app: Option<App>,
    #[prost(message, optional, tag = "2")]
    pub textbox: Option<Textbox>,
    #[prost(message, optional, tag = "3")]
    pub vocabulary: Option<DynamicVocabulary>,
    #[prost(string, tag = "4")]
    pub user_identifier: String,
    #[prost(string, tag = "5")]
    pub content_text: String,
    #[prost(string, tag = "6")]
    pub content_html: String,
    #[prost(message, repeated, tag = "7")]
    pub session_dictation_apps: Vec<App>,
    #[prost(bytes = "vec", tag = "8")]
    pub screenshot: Vec<u8>,
}

#[derive(Clone, PartialEq, ::prost::Message)]
pub struct AudioPackets {
    #[prost(bytes = "vec", repeated, tag = "1")]
    pub items: Vec<Vec<u8>>,
}

#[derive(Clone, PartialEq, ::prost::Message)]
pub struct AudioFile {
    #[prost(bytes = "vec", tag = "1")]
    pub data: Vec<u8>,
}

#[derive(Clone, PartialEq, ::prost::Message)]
pub struct Payload {
    #[prost(oneof = "payload::Audio", tags = "1, 2")]
    pub audio: Option<payload::Audio>,
}

pub mod payload {
    #[derive(Clone, PartialEq, ::prost::Oneof)]
    pub enum Audio {
        #[prost(message, tag = "1")]
        AudioPackets(super::AudioPackets),
        #[prost(message, tag = "2")]
        AudioFile(super::AudioFile),
    }
}

#[derive(Clone, PartialEq, ::prost::Message)]
pub struct Progress {
    #[prost(message, optional, tag = "1")]
    pub transcriber: Option<prost_types::Duration>,
    #[prost(bool, tag = "2")]
    pub transcriber_finished: bool,
    #[prost(message, optional, tag = "3")]
    pub formatter: Option<prost_types::Duration>,
    #[prost(bool, tag = "4")]
    pub formatter_finished: bool,
    #[prost(bool, tag = "5")]
    pub post_processing_finished: bool,
}

#[derive(Clone, PartialEq, ::prost::Message)]
pub struct Text {
    #[prost(string, tag = "1")]
    pub content: String,
    #[prost(uint32, tag = "2")]
    pub num_tokens: u32,
    #[prost(int32, repeated, tag = "3")]
    pub languages: Vec<i32>,
}

#[derive(Clone, PartialEq, ::prost::Message)]
pub struct State {
    #[prost(message, optional, tag = "1")]
    pub progress: Option<Progress>,
    #[prost(message, optional, tag = "2")]
    pub raw_text: Option<Text>,
    #[prost(message, optional, tag = "3")]
    pub formatted_text: Option<Text>,
    #[prost(enumeration = "Confidence", tag = "4")]
    pub confidence: i32,
}

#[derive(Clone, PartialEq, ::prost::Message)]
pub struct Init {
    #[prost(message, optional, tag = "1")]
    pub metadata: Option<Metadata>,
    #[prost(message, optional, tag = "2")]
    pub preferences: Option<Preferences>,
    #[prost(message, optional, tag = "3")]
    pub state: Option<State>,
}

#[derive(Clone, PartialEq, ::prost::Message)]
pub struct Request {
    #[prost(message, optional, tag = "1")]
    pub init: Option<Init>,
    #[prost(message, optional, tag = "2")]
    pub context: Option<Context>,
    #[prost(message, optional, tag = "3")]
    pub payload: Option<Payload>,
    #[prost(enumeration = "Commit", tag = "4")]
    pub commit: i32,
}

#[derive(Clone, PartialEq, ::prost::Message)]
pub struct Transcription {
    #[prost(string, tag = "1")]
    pub html: String,
    #[prost(string, tag = "2")]
    pub plaintext: String,
    #[prost(uint32, tag = "3")]
    pub num_tokens: u32,
    #[prost(int32, repeated, tag = "4")]
    pub languages: Vec<i32>,
    #[prost(float, tag = "5")]
    pub confidence: f32,
    #[prost(bool, tag = "6")]
    pub language_detection_low_confidence: bool,
}

#[derive(Clone, PartialEq, ::prost::Message)]
pub struct ResponseTime {
    #[prost(message, optional, tag = "1")]
    pub transcribe: Option<prost_types::Duration>,
    #[prost(message, optional, tag = "2")]
    pub transcribe_overhead: Option<prost_types::Duration>,
    #[prost(message, optional, tag = "3")]
    pub format: Option<prost_types::Duration>,
    #[prost(message, optional, tag = "4")]
    pub format_overhead: Option<prost_types::Duration>,
    #[prost(message, optional, tag = "5")]
    pub post_process: Option<prost_types::Duration>,
    #[prost(message, optional, tag = "6")]
    pub total: Option<prost_types::Duration>,
    #[prost(message, optional, tag = "7")]
    pub external_asr_time: Option<prost_types::Duration>,
    #[prost(message, optional, tag = "8")]
    pub external_llm_time: Option<prost_types::Duration>,
    #[prost(message, optional, tag = "9")]
    pub generate_queue_duration: Option<prost_types::Duration>,
    #[prost(message, optional, tag = "10")]
    pub generate_work_duration: Option<prost_types::Duration>,
}

#[derive(Clone, PartialEq, ::prost::Message)]
pub struct TranscriptionStats {
    #[prost(uint32, tag = "1")]
    pub word_deletions: u32,
    #[prost(uint32, tag = "2")]
    pub context_word_count: u32,
    #[prost(uint32, tag = "3")]
    pub word_substitutions: u32,
}

#[derive(Clone, PartialEq, ::prost::Message)]
pub struct Result {
    #[prost(message, optional, tag = "1")]
    pub output: Option<Transcription>,
    #[prost(message, optional, tag = "2")]
    pub audio_duration: Option<prost_types::Duration>,
    #[prost(message, optional, tag = "3")]
    pub response_time: Option<ResponseTime>,
    #[prost(enumeration = "SignatureStatus", tag = "4")]
    pub signature_status: i32,
    #[prost(enumeration = "ResultStatus", tag = "5")]
    pub status: i32,
    #[prost(message, optional, tag = "6")]
    pub stats: Option<TranscriptionStats>,
    #[prost(string, tag = "7")]
    pub transcript_origin: String,
    #[prost(string, tag = "8")]
    pub server_address: String,
    #[prost(string, tag = "9")]
    pub format_skip_reason: String,
}

#[derive(Clone, PartialEq, ::prost::Message)]
pub struct Heartbeat {
    #[prost(float, tag = "1")]
    pub audio_received_seconds: f32,
    #[prost(bool, tag = "2")]
    pub commit_ack: bool,
}

#[derive(Clone, PartialEq, ::prost::Message)]
pub struct Response {
    #[prost(message, optional, tag = "1")]
    pub result: Option<Result>,
    #[prost(message, optional, tag = "2")]
    pub state: Option<State>,
    #[prost(message, repeated, tag = "3")]
    pub all_states: Vec<State>,
    #[prost(message, optional, tag = "4")]
    pub heartbeat: Option<Heartbeat>,
}
