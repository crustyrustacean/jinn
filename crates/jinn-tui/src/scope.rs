//! Keymap scopes for context-sensitive key handling.
//!
//! The scope determines which set of keybindings is active.
//! Each sidebar section has its own scope so section-specific keys
//! (like `r` for rename vs pin-relative) are unambiguous.

/// The current keymap context.
///
/// Controls which keybindings are active. Set via
/// [`ratatui_which_key::WhichKeyState::set_scope`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Scope {
    /// Normal mode - navigation and commands.
    Normal,
    /// Sidebar - Persona section.
    SidebarPersona,
    /// Sidebar - Pins section.
    SidebarPins,
    /// Sidebar - Sessions section.
    SidebarSessions,
    /// Sidebar - Task list section.
    SidebarTaskList,
    /// Picker - Provider/model selection.
    PickerProvider,
    /// Picker - Session browser.
    PickerSession,
    /// Picker - Persona selection.
    PickerPersona,
    /// Picker - Theme selection.
    PickerTheme,
    /// Picker - Session lifecycle recipe selection.
    PickerLifecycle,
    /// Picker - Plugin selection.
    PickerPlugin,

    /// Picker - Compaction model selection.
    PickerCompactionModel,
    /// Picker - Tool toggle selection.
    PickerTool,
    /// Picker - Skill toggle selection.
    PickerSkill,
    /// Picker - Read-only task list browser.
    PickerTaskList,
    /// Picker - Curated project directory selection.
    PickerProject,
    /// Input mode - typing into the input buffer.
    Input,
    /// Arg input mode - typing positional args for a lifecycle command.
    ArgInput,
    /// Token budget input mode - typing a numeric budget value.
    TokenBudgetInput,
    /// Rename session input mode - editing a session title.
    RenameSessionInput,
    /// CWD input mode - typing a directory path.
    CwdInput,
    /// Project-add input mode - typing a directory path to register a project.
    ProjectAddInput,
    /// Pruner accumulation threshold input mode - numeric input for the KV-cache gate.
    PrunerAccumulationInput,

    /// Quake bar overlay - global console. Captures all keystrokes while open.
    QuakeBar,

    /// Sidebar resize mode - adjusting sidebar width.
    SidebarResize,
}

impl std::fmt::Display for Scope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Normal => write!(f, "Normal"),
            Self::SidebarPersona => write!(f, "SidebarPersona"),
            Self::SidebarPins => write!(f, "SidebarPins"),
            Self::SidebarSessions => write!(f, "SidebarSessions"),
            Self::SidebarTaskList => write!(f, "SidebarTaskList"),
            Self::PickerProvider => write!(f, "Picker(provider)"),
            Self::PickerSession => write!(f, "Picker(session)"),
            Self::PickerPersona => write!(f, "Picker(persona)"),
            Self::PickerTheme => write!(f, "Picker(theme)"),
            Self::PickerLifecycle => write!(f, "Picker(lifecycle)"),
            Self::PickerPlugin => write!(f, "Picker(plugin)"),

            Self::PickerCompactionModel => write!(f, "Picker(compaction-model)"),
            Self::PickerTool => write!(f, "Picker(tool)"),
            Self::PickerSkill => write!(f, "Picker(skill)"),
            Self::PickerTaskList => write!(f, "Picker(task-list)"),
            Self::PickerProject => write!(f, "Picker(project)"),
            Self::Input => write!(f, "Input"),
            Self::ArgInput => write!(f, "ArgInput"),
            Self::TokenBudgetInput => write!(f, "TokenBudgetInput"),
            Self::SidebarResize => write!(f, "SidebarResize"),
            Self::RenameSessionInput => write!(f, "RenameSessionInput"),
            Self::CwdInput => write!(f, "CwdInput"),
            Self::ProjectAddInput => write!(f, "ProjectAddInput"),
            Self::PrunerAccumulationInput => write!(f, "PrunerAccumulationInput"),
            Self::QuakeBar => write!(f, "QuakeBar"),
        }
    }
}

impl std::str::FromStr for Scope {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "Normal" => Ok(Self::Normal),
            "SidebarPersona" => Ok(Self::SidebarPersona),
            "SidebarPins" => Ok(Self::SidebarPins),
            "SidebarSessions" => Ok(Self::SidebarSessions),
            "SidebarTaskList" => Ok(Self::SidebarTaskList),
            "Picker(provider)" => Ok(Self::PickerProvider),
            "Picker(session)" => Ok(Self::PickerSession),
            "Picker(persona)" => Ok(Self::PickerPersona),
            "Picker(theme)" => Ok(Self::PickerTheme),
            "Picker(lifecycle)" => Ok(Self::PickerLifecycle),
            "Picker(plugin)" => Ok(Self::PickerPlugin),

            "Picker(compaction-model)" => Ok(Self::PickerCompactionModel),
            "Picker(tool)" => Ok(Self::PickerTool),
            "Picker(skill)" => Ok(Self::PickerSkill),
            "Picker(task-list)" => Ok(Self::PickerTaskList),
            "Picker(project)" => Ok(Self::PickerProject),
            "Input" => Ok(Self::Input),
            "ArgInput" => Ok(Self::ArgInput),
            "TokenBudgetInput" => Ok(Self::TokenBudgetInput),
            "RenameSessionInput" => Ok(Self::RenameSessionInput),
            "CwdInput" => Ok(Self::CwdInput),
            "ProjectAddInput" => Ok(Self::ProjectAddInput),
            "PrunerAccumulationInput" => Ok(Self::PrunerAccumulationInput),
            "QuakeBar" => Ok(Self::QuakeBar),
            "SidebarResize" => Ok(Self::SidebarResize),

            _ => Err(()),
        }
    }
}
