use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum SettingsPatch {
    EditorTabSize {
        value: u8,
    },
    EditorInsertSpaces {
        value: bool,
    },
    TerminalField {
        field: SettingsTerminalField,
        #[serde(deserialize_with = "required_scalar")]
        #[schemars(with = "RequiredSettingsScalar")]
        value: Option<SettingsScalar>,
    },
    KeybindingSet {
        action: String,
        #[serde(deserialize_with = "required_shortcut")]
        #[schemars(with = "RequiredShortcut")]
        shortcut: Option<String>,
    },
    KeybindingReset {
        action: String,
    },
    KeybindsFocusFollowsPointer {
        value: bool,
    },
    ThemeBuiltin {
        value: SettingsBuiltinTheme,
    },
    ThemeAppearance {
        value: SettingsAppearance,
    },
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum SettingsBuiltinTheme {
    Lomi,
    Deepmono,
}
impl SettingsBuiltinTheme {
    pub fn id(self) -> Option<String> {
        match self {
            Self::Lomi => None,
            Self::Deepmono => Some("@builtin-deepmono".into()),
        }
    }
}
fn required_shortcut<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<String>, D::Error> {
    Option::<String>::deserialize(deserializer)
}
struct RequiredShortcut;
impl JsonSchema for RequiredShortcut {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "RequiredShortcut".into()
    }
    fn json_schema(generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        Option::<String>::json_schema(generator)
    }
}
fn required_scalar<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<SettingsScalar>, D::Error> {
    Option::<SettingsScalar>::deserialize(deserializer)
}
struct RequiredSettingsScalar;
impl JsonSchema for RequiredSettingsScalar {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "RequiredSettingsScalar".into()
    }
    fn json_schema(generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        Option::<SettingsScalar>::json_schema(generator)
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SettingsEditorValues {
    pub tab_size: u8,
    pub insert_spaces: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SettingsUpdateInput {
    pub workspace_id: String,
    pub patch: SettingsPatch,
    pub expected_settings_revision: String,
    pub expected_revision: String,
    pub retry_epoch: String,
    pub request_key: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SettingsUpdateCommand {
    pub workspace_id: String,
    pub input: SettingsUpdateInput,
    pub not_after_millis: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SettingsUpdated {
    pub workspace_id: String,
    pub section: SettingsSection,
    pub previous_stored_revision: Option<String>,
    pub stored_revision: String,
    pub applied: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum SettingsPage {
    Keybinds,
    Themes,
    Plugins,
    Terminal,
    About,
    ChatAi,
    Android,
    AgentControl,
}
impl SettingsPage {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Keybinds => "keybinds",
            Self::Themes => "themes",
            Self::Plugins => "plugins",
            Self::Terminal => "terminal",
            Self::About => "about",
            Self::ChatAi => "chat-ai",
            Self::Android => "android",
            Self::AgentControl => "agent-control",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SettingsOpenInput {
    pub workspace_id: String,
    pub page: SettingsPage,
    pub expected_revision: String,
    pub retry_epoch: String,
    pub request_key: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SettingsOpenCommand {
    pub workspace_id: String,
    pub page: SettingsPage,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SettingsOpened {
    pub workspace_id: String,
    pub page: SettingsPage,
    /// The native window accepted this request; no preference was changed.
    pub requested: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SettingsSection {
    Editor,
    Terminal,
    Keybinds,
    Themes,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SettingsReadInput {
    pub workspace_id: String,
    pub section: SettingsSection,
    #[serde(default)]
    pub offset: u16,
    #[serde(default = "settings_page_limit")]
    pub limit: u16,
    pub expected_revision: Option<String>,
}
fn settings_page_limit() -> u16 {
    100
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SettingsReadiness {
    Ready,
    RecoveryRequired,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SettingsSnapshot {
    pub workspace_id: String,
    pub section: SettingsSection,
    pub revision: String,
    pub readiness: SettingsReadiness,
    pub values: SettingsValues,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(
    tag = "section",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum SettingsValues {
    Editor {
        tab_size: u8,
        insert_spaces: bool,
    },
    Terminal {
        appearance_overrides: Box<SettingsTerminalAppearance>,
        behavior: SettingsTerminalBehavior,
        windows_shell: SettingsWindowsShell,
        agent_notifications: bool,
        always_show_titles: bool,
    },
    Keybinds {
        focus_follows_pointer: bool,
        items: Vec<SettingsBinding>,
        total: u16,
        offset: u16,
        next_offset: Option<u16>,
    },
    Themes {
        active: Option<String>,
        file_icons: Option<String>,
        product_icons: Option<String>,
        appearance: SettingsAppearance,
        effective_appearance: SettingsEffectiveAppearance,
        safe_mode: bool,
    },
}
impl SettingsValues {
    pub fn section(&self) -> SettingsSection {
        match self {
            Self::Editor { .. } => SettingsSection::Editor,
            Self::Terminal { .. } => SettingsSection::Terminal,
            Self::Keybinds { .. } => SettingsSection::Keybinds,
            Self::Themes { .. } => SettingsSection::Themes,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SettingsBinding {
    pub action: String,
    pub shortcut: Option<String>,
    pub default_shortcut: Option<String>,
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum SettingsWindowsShell {
    Powershell,
    Cmd,
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum SettingsAppearance {
    System,
    Light,
    Dark,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum SettingsEffectiveAppearance {
    Light,
    Dark,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SettingsTerminalBehavior {
    pub scrollback: u32,
    pub scroll_sensitivity: f64,
    pub fast_scroll_sensitivity: f64,
    pub smooth_scroll_duration: u16,
    pub tab_stop_width: u8,
    pub scroll_on_user_input: bool,
    pub scroll_on_erase_in_display: bool,
    pub alt_click_moves_cursor: bool,
    pub right_click_selects_word: bool,
    pub mac_option_is_meta: bool,
    pub mac_option_click_forces_selection: bool,
    pub screen_reader_mode: bool,
    pub custom_glyphs: bool,
    pub rescale_overlapping_glyphs: bool,
    pub word_separator: String,
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum SettingsCursor {
    Bar,
    Block,
    Underline,
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum SettingsInactiveCursor {
    Outline,
    Bar,
    Block,
    Underline,
    None,
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum SettingsWeightName {
    Normal,
    Bold,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum SettingsWeight {
    Named(SettingsWeightName),
    Numeric(f64),
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SettingsTerminalAppearance {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font_family: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font_size: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font_weight: Option<SettingsWeight>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font_weight_bold: Option<SettingsWeight>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line_height: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub letter_spacing: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cursor_width: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cursor_style: Option<SettingsCursor>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cursor_inactive_style: Option<SettingsInactiveCursor>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cursor_blink: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub minimum_contrast_ratio: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub draw_bold_text_in_bright_colors: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub colors: Option<SettingsTerminalColors>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SettingsReadRequest {
    pub request_id: String,
    pub ui_epoch: String,
    pub project_id: String,
    pub input: SettingsReadInput,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SettingsReadReply {
    pub request_id: String,
    pub ui_epoch: String,
    pub snapshot: Option<SettingsSnapshot>,
    pub error: Option<crate::ErrorCode>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SettingsTerminalColors {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub background: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub foreground: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cursor_accent: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selection_background: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selection_foreground: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selection_inactive_background: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub black: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub red: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub green: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub yellow: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blue: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub magenta: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cyan: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub white: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bright_black: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bright_red: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bright_green: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bright_yellow: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bright_blue: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bright_magenta: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bright_cyan: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bright_white: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub search_match_background: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub search_active_match_background: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub search_match_border: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub search_active_match_border: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum SettingsScalar {
    Boolean(bool),
    Number(f64),
    Text(String),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum SettingsTerminalField {
    #[serde(rename = "appearance.fontFamily")]
    AppearanceFontFamily,
    #[serde(rename = "appearance.fontSize")]
    AppearanceFontSize,
    #[serde(rename = "appearance.fontWeight")]
    AppearanceFontWeight,
    #[serde(rename = "appearance.fontWeightBold")]
    AppearanceFontWeightBold,
    #[serde(rename = "appearance.lineHeight")]
    AppearanceLineHeight,
    #[serde(rename = "appearance.letterSpacing")]
    AppearanceLetterSpacing,
    #[serde(rename = "appearance.cursorWidth")]
    AppearanceCursorWidth,
    #[serde(rename = "appearance.cursorStyle")]
    AppearanceCursorStyle,
    #[serde(rename = "appearance.cursorInactiveStyle")]
    AppearanceCursorInactiveStyle,
    #[serde(rename = "appearance.cursorBlink")]
    AppearanceCursorBlink,
    #[serde(rename = "appearance.minimumContrastRatio")]
    AppearanceMinimumContrastRatio,
    #[serde(rename = "appearance.drawBoldTextInBrightColors")]
    AppearanceDrawBoldTextInBrightColors,
    #[serde(rename = "appearance.colors.background")]
    AppearanceColorsBackground,
    #[serde(rename = "appearance.colors.foreground")]
    AppearanceColorsForeground,
    #[serde(rename = "appearance.colors.cursor")]
    AppearanceColorsCursor,
    #[serde(rename = "appearance.colors.cursorAccent")]
    AppearanceColorsCursorAccent,
    #[serde(rename = "appearance.colors.selectionBackground")]
    AppearanceColorsSelectionBackground,
    #[serde(rename = "appearance.colors.selectionForeground")]
    AppearanceColorsSelectionForeground,
    #[serde(rename = "appearance.colors.selectionInactiveBackground")]
    AppearanceColorsSelectionInactiveBackground,
    #[serde(rename = "appearance.colors.black")]
    AppearanceColorsBlack,
    #[serde(rename = "appearance.colors.red")]
    AppearanceColorsRed,
    #[serde(rename = "appearance.colors.green")]
    AppearanceColorsGreen,
    #[serde(rename = "appearance.colors.yellow")]
    AppearanceColorsYellow,
    #[serde(rename = "appearance.colors.blue")]
    AppearanceColorsBlue,
    #[serde(rename = "appearance.colors.magenta")]
    AppearanceColorsMagenta,
    #[serde(rename = "appearance.colors.cyan")]
    AppearanceColorsCyan,
    #[serde(rename = "appearance.colors.white")]
    AppearanceColorsWhite,
    #[serde(rename = "appearance.colors.brightBlack")]
    AppearanceColorsBrightBlack,
    #[serde(rename = "appearance.colors.brightRed")]
    AppearanceColorsBrightRed,
    #[serde(rename = "appearance.colors.brightGreen")]
    AppearanceColorsBrightGreen,
    #[serde(rename = "appearance.colors.brightYellow")]
    AppearanceColorsBrightYellow,
    #[serde(rename = "appearance.colors.brightBlue")]
    AppearanceColorsBrightBlue,
    #[serde(rename = "appearance.colors.brightMagenta")]
    AppearanceColorsBrightMagenta,
    #[serde(rename = "appearance.colors.brightCyan")]
    AppearanceColorsBrightCyan,
    #[serde(rename = "appearance.colors.brightWhite")]
    AppearanceColorsBrightWhite,
    #[serde(rename = "appearance.colors.searchMatchBackground")]
    AppearanceColorsSearchMatchBackground,
    #[serde(rename = "appearance.colors.searchActiveMatchBackground")]
    AppearanceColorsSearchActiveMatchBackground,
    #[serde(rename = "appearance.colors.searchMatchBorder")]
    AppearanceColorsSearchMatchBorder,
    #[serde(rename = "appearance.colors.searchActiveMatchBorder")]
    AppearanceColorsSearchActiveMatchBorder,
    #[serde(rename = "behavior.scrollback")]
    BehaviorScrollback,
    #[serde(rename = "behavior.scrollSensitivity")]
    BehaviorScrollSensitivity,
    #[serde(rename = "behavior.fastScrollSensitivity")]
    BehaviorFastScrollSensitivity,
    #[serde(rename = "behavior.smoothScrollDuration")]
    BehaviorSmoothScrollDuration,
    #[serde(rename = "behavior.tabStopWidth")]
    BehaviorTabStopWidth,
    #[serde(rename = "behavior.scrollOnUserInput")]
    BehaviorScrollOnUserInput,
    #[serde(rename = "behavior.scrollOnEraseInDisplay")]
    BehaviorScrollOnEraseInDisplay,
    #[serde(rename = "behavior.altClickMovesCursor")]
    BehaviorAltClickMovesCursor,
    #[serde(rename = "behavior.rightClickSelectsWord")]
    BehaviorRightClickSelectsWord,
    #[serde(rename = "behavior.macOptionIsMeta")]
    BehaviorMacOptionIsMeta,
    #[serde(rename = "behavior.macOptionClickForcesSelection")]
    BehaviorMacOptionClickForcesSelection,
    #[serde(rename = "behavior.screenReaderMode")]
    BehaviorScreenReaderMode,
    #[serde(rename = "behavior.customGlyphs")]
    BehaviorCustomGlyphs,
    #[serde(rename = "behavior.rescaleOverlappingGlyphs")]
    BehaviorRescaleOverlappingGlyphs,
    #[serde(rename = "behavior.wordSeparator")]
    BehaviorWordSeparator,
    #[serde(rename = "windowsShell")]
    WindowsShell,
    #[serde(rename = "agentNotifications")]
    AgentNotifications,
    #[serde(rename = "alwaysShowTitles")]
    AlwaysShowTitles,
}
impl SettingsTerminalField {
    pub fn path(self) -> &'static str {
        match self {
            Self::AppearanceFontFamily => "appearance.fontFamily",
            Self::AppearanceFontSize => "appearance.fontSize",
            Self::AppearanceFontWeight => "appearance.fontWeight",
            Self::AppearanceFontWeightBold => "appearance.fontWeightBold",
            Self::AppearanceLineHeight => "appearance.lineHeight",
            Self::AppearanceLetterSpacing => "appearance.letterSpacing",
            Self::AppearanceCursorWidth => "appearance.cursorWidth",
            Self::AppearanceCursorStyle => "appearance.cursorStyle",
            Self::AppearanceCursorInactiveStyle => "appearance.cursorInactiveStyle",
            Self::AppearanceCursorBlink => "appearance.cursorBlink",
            Self::AppearanceMinimumContrastRatio => "appearance.minimumContrastRatio",
            Self::AppearanceDrawBoldTextInBrightColors => "appearance.drawBoldTextInBrightColors",
            Self::AppearanceColorsBackground => "appearance.colors.background",
            Self::AppearanceColorsForeground => "appearance.colors.foreground",
            Self::AppearanceColorsCursor => "appearance.colors.cursor",
            Self::AppearanceColorsCursorAccent => "appearance.colors.cursorAccent",
            Self::AppearanceColorsSelectionBackground => "appearance.colors.selectionBackground",
            Self::AppearanceColorsSelectionForeground => "appearance.colors.selectionForeground",
            Self::AppearanceColorsSelectionInactiveBackground => {
                "appearance.colors.selectionInactiveBackground"
            }
            Self::AppearanceColorsBlack => "appearance.colors.black",
            Self::AppearanceColorsRed => "appearance.colors.red",
            Self::AppearanceColorsGreen => "appearance.colors.green",
            Self::AppearanceColorsYellow => "appearance.colors.yellow",
            Self::AppearanceColorsBlue => "appearance.colors.blue",
            Self::AppearanceColorsMagenta => "appearance.colors.magenta",
            Self::AppearanceColorsCyan => "appearance.colors.cyan",
            Self::AppearanceColorsWhite => "appearance.colors.white",
            Self::AppearanceColorsBrightBlack => "appearance.colors.brightBlack",
            Self::AppearanceColorsBrightRed => "appearance.colors.brightRed",
            Self::AppearanceColorsBrightGreen => "appearance.colors.brightGreen",
            Self::AppearanceColorsBrightYellow => "appearance.colors.brightYellow",
            Self::AppearanceColorsBrightBlue => "appearance.colors.brightBlue",
            Self::AppearanceColorsBrightMagenta => "appearance.colors.brightMagenta",
            Self::AppearanceColorsBrightCyan => "appearance.colors.brightCyan",
            Self::AppearanceColorsBrightWhite => "appearance.colors.brightWhite",
            Self::AppearanceColorsSearchMatchBackground => {
                "appearance.colors.searchMatchBackground"
            }
            Self::AppearanceColorsSearchActiveMatchBackground => {
                "appearance.colors.searchActiveMatchBackground"
            }
            Self::AppearanceColorsSearchMatchBorder => "appearance.colors.searchMatchBorder",
            Self::AppearanceColorsSearchActiveMatchBorder => {
                "appearance.colors.searchActiveMatchBorder"
            }
            Self::BehaviorScrollback => "behavior.scrollback",
            Self::BehaviorScrollSensitivity => "behavior.scrollSensitivity",
            Self::BehaviorFastScrollSensitivity => "behavior.fastScrollSensitivity",
            Self::BehaviorSmoothScrollDuration => "behavior.smoothScrollDuration",
            Self::BehaviorTabStopWidth => "behavior.tabStopWidth",
            Self::BehaviorScrollOnUserInput => "behavior.scrollOnUserInput",
            Self::BehaviorScrollOnEraseInDisplay => "behavior.scrollOnEraseInDisplay",
            Self::BehaviorAltClickMovesCursor => "behavior.altClickMovesCursor",
            Self::BehaviorRightClickSelectsWord => "behavior.rightClickSelectsWord",
            Self::BehaviorMacOptionIsMeta => "behavior.macOptionIsMeta",
            Self::BehaviorMacOptionClickForcesSelection => "behavior.macOptionClickForcesSelection",
            Self::BehaviorScreenReaderMode => "behavior.screenReaderMode",
            Self::BehaviorCustomGlyphs => "behavior.customGlyphs",
            Self::BehaviorRescaleOverlappingGlyphs => "behavior.rescaleOverlappingGlyphs",
            Self::BehaviorWordSeparator => "behavior.wordSeparator",
            Self::WindowsShell => "windowsShell",
            Self::AgentNotifications => "agentNotifications",
            Self::AlwaysShowTitles => "alwaysShowTitles",
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SettingsTerminalValues {
    pub appearance: SettingsTerminalAppearance,
    pub behavior: SettingsTerminalBehavior,
    pub windows_shell: SettingsWindowsShell,
    pub agent_notifications: bool,
    pub always_show_titles: bool,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SettingsUpdateValues {
    Editor(SettingsEditorValues),
    Terminal(Box<SettingsTerminalValues>),
    Keybinds(SettingsKeybindsValues),
    Themes(SettingsThemeValues),
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SettingsThemeValues {
    pub active: Option<String>,
    pub file_icons: Option<String>,
    pub product_icons: Option<String>,
    pub appearance: SettingsAppearance,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SettingsKeybindsValues {
    pub focus_follows_pointer: bool,
    pub action: Option<SettingsKeybindingAction>,
    pub source_revision: Option<String>,
    pub definitions_revision: String,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SettingsKeybindingAction {
    pub id: String,
    pub label: String,
    pub shortcut: Option<String>,
    pub default_shortcut: Option<String>,
}
impl From<SettingsEditorValues> for SettingsUpdateValues {
    fn from(value: SettingsEditorValues) -> Self {
        Self::Editor(value)
    }
}
impl SettingsPatch {
    pub fn section(&self) -> SettingsSection {
        match self {
            Self::EditorTabSize { .. } | Self::EditorInsertSpaces { .. } => SettingsSection::Editor,
            Self::TerminalField { .. } => SettingsSection::Terminal,
            Self::KeybindingSet { .. }
            | Self::KeybindingReset { .. }
            | Self::KeybindsFocusFollowsPointer { .. } => SettingsSection::Keybinds,
            Self::ThemeBuiltin { .. } | Self::ThemeAppearance { .. } => SettingsSection::Themes,
        }
    }
    pub fn apply_values(&self, current: &mut SettingsUpdateValues) -> Result<(), crate::ErrorCode> {
        match (self, current) {
            (Self::ThemeBuiltin { value }, SettingsUpdateValues::Themes(current)) => {
                current.active = value.id()
            }
            (Self::ThemeAppearance { value }, SettingsUpdateValues::Themes(current)) => {
                if current
                    .active
                    .as_deref()
                    .is_some_and(|id| id != "@builtin-deepmono")
                {
                    return Err(crate::ErrorCode::UnsupportedCapability);
                }
                current.appearance = *value;
            }
            (Self::EditorTabSize { value }, SettingsUpdateValues::Editor(current)) => {
                current.tab_size = *value
            }
            (Self::EditorInsertSpaces { value }, SettingsUpdateValues::Editor(current)) => {
                current.insert_spaces = *value
            }
            (Self::TerminalField { .. }, SettingsUpdateValues::Terminal(current)) => {
                let mut json = serde_json::to_value(&**current)
                    .map_err(|_| crate::ErrorCode::ResourceExhausted)?;
                self.apply_terminal(&mut json)?;
                **current = serde_json::from_value(json)
                    .map_err(|_| crate::ErrorCode::ResourceExhausted)?;
            }
            (Self::KeybindingSet { action, shortcut }, SettingsUpdateValues::Keybinds(current)) => {
                let target = current
                    .action
                    .as_mut()
                    .filter(|a| &a.id == action)
                    .ok_or(crate::ErrorCode::RevisionConflict)?;
                target.shortcut = shortcut.clone();
            }
            (Self::KeybindingReset { action }, SettingsUpdateValues::Keybinds(current)) => {
                let target = current
                    .action
                    .as_mut()
                    .filter(|a| &a.id == action)
                    .ok_or(crate::ErrorCode::RevisionConflict)?;
                target.shortcut = target.default_shortcut.clone();
            }
            (
                Self::KeybindsFocusFollowsPointer { value },
                SettingsUpdateValues::Keybinds(current),
            ) => {
                if current.action.is_some() {
                    return Err(crate::ErrorCode::RevisionConflict);
                }
                current.focus_follows_pointer = *value;
            }
            _ => return Err(crate::ErrorCode::UnsupportedCapability),
        }
        Ok(())
    }
    /// Apply one allowlisted leaf to a native terminal preference document.
    /// Section-specific validators still check ranges and accepted values.
    pub fn apply_terminal(&self, current: &mut serde_json::Value) -> Result<(), crate::ErrorCode> {
        let Self::TerminalField { field, value } = self else {
            return Err(crate::ErrorCode::UnsupportedCapability);
        };
        let path = field.path();
        if value.is_none() && !path.starts_with("appearance.") {
            return Err(crate::ErrorCode::ResourceExhausted);
        }
        let mut segments = path.split('.').peekable();
        let mut at = current;
        while let Some(key) = segments.next() {
            let object = at
                .as_object_mut()
                .ok_or(crate::ErrorCode::ResourceExhausted)?;
            if segments.peek().is_none() {
                match value {
                    Some(value) => {
                        let json = match value {
                            SettingsScalar::Number(number) if !number.is_finite() => {
                                return Err(crate::ErrorCode::ResourceExhausted)
                            }
                            SettingsScalar::Number(number)
                                if number.fract() == 0.0
                                    && number.abs() <= 9_007_199_254_740_991.0 =>
                            {
                                serde_json::json!(*number as i64)
                            }
                            _ => serde_json::to_value(value)
                                .map_err(|_| crate::ErrorCode::ResourceExhausted)?,
                        };
                        object.insert(key.into(), json);
                    }
                    None => {
                        object.remove(key);
                    }
                }
                return Ok(());
            }
            at = object.entry(key).or_insert_with(|| serde_json::json!({}));
        }
        Err(crate::ErrorCode::ResourceExhausted)
    }
}

#[cfg(test)]
mod update_tests {
    use super::*;
    #[test]
    fn builtin_theme_changes_preserve_icons_and_refuse_custom_appearance_activation() {
        let mut values = SettingsUpdateValues::Themes(SettingsThemeValues {
            active: Some("user-theme".into()),
            file_icons: Some("icons".into()),
            product_icons: None,
            appearance: SettingsAppearance::System,
        });
        let before = values.clone();
        assert_eq!(
            SettingsPatch::ThemeAppearance {
                value: SettingsAppearance::Dark
            }
            .apply_values(&mut values),
            Err(crate::ErrorCode::UnsupportedCapability)
        );
        assert_eq!(values, before);
        SettingsPatch::ThemeBuiltin {
            value: SettingsBuiltinTheme::Deepmono,
        }
        .apply_values(&mut values)
        .unwrap();
        SettingsPatch::ThemeAppearance {
            value: SettingsAppearance::Light,
        }
        .apply_values(&mut values)
        .unwrap();
        let SettingsUpdateValues::Themes(value) = values else {
            panic!()
        };
        assert_eq!(value.active.as_deref(), Some("@builtin-deepmono"));
        assert_eq!(value.file_icons.as_deref(), Some("icons"));
        assert_eq!(value.appearance, SettingsAppearance::Light);
    }
    #[test]
    fn shortcut_disable_requires_explicit_null_and_reset_has_no_value() {
        use serde_json::json;
        assert!(serde_json::from_value::<SettingsPatch>(
            json!({"type":"keybinding_set","action":"saveFile","shortcut":null})
        )
        .is_ok());
        assert!(serde_json::from_value::<SettingsPatch>(
            json!({"type":"keybinding_set","action":"saveFile"})
        )
        .is_err());
        assert!(serde_json::from_value::<SettingsPatch>(
            json!({"type":"keybinding_reset","action":"saveFile","shortcut":null})
        )
        .is_err());
    }
    #[test]
    fn terminal_patch_requires_an_explicit_nullable_scalar_and_closed_field() {
        use serde_json::json;
        let patch = json!({"type":"terminal_field","field":"appearance.fontSize","value":null});
        assert!(serde_json::from_value::<SettingsPatch>(patch.clone()).is_ok());
        let mut missing = patch.clone();
        missing.as_object_mut().unwrap().remove("value");
        assert!(serde_json::from_value::<SettingsPatch>(missing).is_err());
        for invalid in [
            json!({"type":"terminal_field","field":"profile.command","value":"shell"}),
            json!({"type":"terminal_field","field":"appearance.fontSize","value":{"other":18}}),
        ] {
            assert!(serde_json::from_value::<SettingsPatch>(invalid).is_err());
        }
    }
}
