// SPDX-License-Identifier: LGPL-2.1-or-later
//! mica — diagnostic types and rendering

use std::fmt;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Code {
    EParse,
    EDup,
    EUnknownRef,
    EType,
    ESelectCycle,
    EDepCycle,
    EUnknownOverride,
    EComputedOverride,
    ERange,
    EHiddenOverride,
    EChoiceMulti,
    ESelectHidden,
    ESelectChoiceConflict,
    ESelectVsOverride,
    EUnstable,
    EUnknownMenu,
    EMenuCycle,
    ERandUnsat,
    WChoiceRestored,
}

impl Code {
    pub fn as_str(self) -> &'static str {
        match self {
            Code::EParse => "E_PARSE",
            Code::EDup => "E_DUP",
            Code::EUnknownRef => "E_UNKNOWN_REF",
            Code::EType => "E_TYPE",
            Code::ESelectCycle => "E_SELECT_CYCLE",
            Code::EDepCycle => "E_DEP_CYCLE",
            Code::EUnknownOverride => "E_UNKNOWN_OVERRIDE",
            Code::EComputedOverride => "E_COMPUTED_OVERRIDE",
            Code::ERange => "E_RANGE",
            Code::EHiddenOverride => "E_HIDDEN_OVERRIDE",
            Code::EChoiceMulti => "E_CHOICE_MULTI",
            Code::ESelectHidden => "E_SELECT_HIDDEN",
            Code::ESelectChoiceConflict => "E_SELECT_CHOICE_CONFLICT",
            Code::ESelectVsOverride => "E_SELECT_VS_OVERRIDE",
            Code::EUnstable => "E_UNSTABLE",
            Code::EUnknownMenu => "E_UNKNOWN_MENU",
            Code::EMenuCycle => "E_MENU_CYCLE",
            Code::ERandUnsat => "E_RAND_UNSAT",
            Code::WChoiceRestored => "W_CHOICE_RESTORED",
        }
    }

    pub fn is_warning(self) -> bool {
        matches!(self, Code::WChoiceRestored)
    }
}

#[derive(Debug, Clone)]
pub struct Diagnostic {
    pub code: Code,
    pub message: String,
    pub file: Option<PathBuf>,
    pub line: Option<u32>,
    /// Select chain (source → … → target) for select-related errors.
    pub chain: Vec<String>,
}

impl Diagnostic {
    pub fn new(code: Code, message: impl Into<String>) -> Self {
        Diagnostic {
            code,
            message: message.into(),
            file: None,
            line: None,
            chain: Vec::new(),
        }
    }

    pub fn at(mut self, file: &Path, line: u32) -> Self {
        self.file = Some(file.to_path_buf());
        self.line = Some(line);
        self
    }

    pub fn with_chain(mut self, chain: Vec<String>) -> Self {
        self.chain = chain;
        self
    }
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let kind = if self.code.is_warning() {
            "warning"
        } else {
            "error"
        };
        write!(f, "{}[{}]: {}", kind, self.code.as_str(), self.message)?;
        if let Some(file) = &self.file {
            write!(f, " at {}", file.display())?;
            if let Some(line) = self.line {
                write!(f, ":{}", line)?;
            }
        }
        if !self.chain.is_empty() {
            write!(f, "; select chain: {}", self.chain.join(" -> "))?;
        }
        Ok(())
    }
}
