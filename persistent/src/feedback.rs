//! Structured feedback messages for persistence operations.

extern crate alloc;
use alloc::string::String;
use alloc::vec::Vec;

#[derive(Debug, Clone)]
pub struct FeedbackMessage {
    pub level: FeedbackLevel,
    pub category: FeedbackCategory,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeedbackLevel {
    Info,
    Success,
    Warning,
    Error,
    Debug,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeedbackCategory {
    PeHeader,
    Section,
    Relocation,
    Memory,
    Storage,
    Verification,
    General,
}

impl FeedbackMessage {
    pub fn info(category: FeedbackCategory, message: impl Into<String>) -> Self {
        Self {
            level: FeedbackLevel::Info,
            category,
            message: message.into(),
        }
    }

    pub fn success(category: FeedbackCategory, message: impl Into<String>) -> Self {
        Self {
            level: FeedbackLevel::Success,
            category,
            message: message.into(),
        }
    }

    pub fn warning(category: FeedbackCategory, message: impl Into<String>) -> Self {
        Self {
            level: FeedbackLevel::Warning,
            category,
            message: message.into(),
        }
    }

    pub fn error(category: FeedbackCategory, message: impl Into<String>) -> Self {
        Self {
            level: FeedbackLevel::Error,
            category,
            message: message.into(),
        }
    }

    pub fn debug(category: FeedbackCategory, message: impl Into<String>) -> Self {
        Self {
            level: FeedbackLevel::Debug,
            category,
            message: message.into(),
        }
    }

    pub fn format_line(&self) -> String {
        use alloc::format;
        let prefix = match self.level {
            FeedbackLevel::Info => "[INFO]",
            FeedbackLevel::Success => "[OK]",
            FeedbackLevel::Warning => "[WARN]",
            FeedbackLevel::Error => "[ERR]",
            FeedbackLevel::Debug => "[DBG]",
        };
        format!("{} {}", prefix, self.message)
    }
}

/// Bounded FIFO ring of messages for batch display.
pub struct FeedbackCollector {
    messages: Vec<FeedbackMessage>,
    max_messages: usize,
}

impl FeedbackCollector {
    pub fn new(max_messages: usize) -> Self {
        Self {
            messages: Vec::with_capacity(max_messages),
            max_messages,
        }
    }

    pub fn add(&mut self, msg: FeedbackMessage) {
        if self.messages.len() >= self.max_messages {
            self.messages.remove(0);
        }
        self.messages.push(msg);
    }

    pub fn info(&mut self, category: FeedbackCategory, message: impl Into<String>) {
        self.add(FeedbackMessage::info(category, message));
    }

    pub fn success(&mut self, category: FeedbackCategory, message: impl Into<String>) {
        self.add(FeedbackMessage::success(category, message));
    }

    pub fn warning(&mut self, category: FeedbackCategory, message: impl Into<String>) {
        self.add(FeedbackMessage::warning(category, message));
    }

    pub fn error(&mut self, category: FeedbackCategory, message: impl Into<String>) {
        self.add(FeedbackMessage::error(category, message));
    }

    pub fn debug(&mut self, category: FeedbackCategory, message: impl Into<String>) {
        self.add(FeedbackMessage::debug(category, message));
    }

    pub fn messages(&self) -> &[FeedbackMessage] {
        &self.messages
    }

    pub fn messages_by_level(&self, level: FeedbackLevel) -> Vec<&FeedbackMessage> {
        self.messages.iter().filter(|m| m.level == level).collect()
    }

    pub fn messages_by_category(&self, category: FeedbackCategory) -> Vec<&FeedbackMessage> {
        self.messages
            .iter()
            .filter(|m| m.category == category)
            .collect()
    }

    pub fn clear(&mut self) {
        self.messages.clear();
    }

    pub fn has_errors(&self) -> bool {
        self.messages
            .iter()
            .any(|m| m.level == FeedbackLevel::Error)
    }
}

/// PE header summary for TUI display.
pub struct PeDumpSummary {
    pub arch: String,
    pub image_base_header: u64,
    pub actual_load_address: u64,
    pub relocation_delta: i64,
    pub num_sections: u16,
    pub has_reloc_section: bool,
    pub reloc_section_rva: Option<u32>,
    pub reloc_section_size: Option<u32>,
    pub size_of_image: u32,
}

impl PeDumpSummary {
    pub fn from_headers(
        headers: &crate::pe::header::PeHeaders,
        actual_load_address: u64,
        reloc_rva: Option<u32>,
        reloc_size: Option<u32>,
    ) -> Self {
        Self {
            arch: headers.coff.machine_name().into(),
            image_base_header: headers.optional.image_base,
            actual_load_address,
            relocation_delta: headers.relocation_delta(actual_load_address),
            num_sections: headers.coff.number_of_sections,
            has_reloc_section: reloc_rva.is_some(),
            reloc_section_rva: reloc_rva,
            reloc_section_size: reloc_size,
            size_of_image: headers.optional.size_of_image,
        }
    }

    pub fn format_lines(&self) -> Vec<String> {
        use alloc::format;
        let mut lines = Vec::new();

        lines.push(format!("Architecture: {}", self.arch));
        lines.push(format!(
            "ImageBase (header): 0x{:016X}",
            self.image_base_header
        ));
        lines.push(format!("Loaded at: 0x{:016X}", self.actual_load_address));
        lines.push(format!(
            "Relocation delta: 0x{:016X}",
            self.relocation_delta as u64
        ));
        lines.push(format!("Sections: {}", self.num_sections));
        lines.push(format!("Image size: {} bytes", self.size_of_image));

        if self.has_reloc_section {
            if let (Some(rva), Some(size)) = (self.reloc_section_rva, self.reloc_section_size) {
                lines.push(format!(".reloc @ RVA 0x{:X} ({} bytes)", rva, size));
            }
        } else {
            lines.push(".reloc section: NOT FOUND".into());
        }

        lines
    }
}
