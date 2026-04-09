//! US keyboard layout – maps key names to CDP key descriptions.
//!
//! Direct port of Puppeteer's `USKeyboardLayout.ts`.

use std::collections::HashMap;
use std::sync::LazyLock;

/// Raw definition of a key, mirroring Puppeteer's `KeyDefinition`.
#[derive(Debug, Clone)]
pub struct KeyDefinition {
    pub key_code: i64,
    pub shift_key_code: Option<i64>,
    pub key: &'static str,
    pub shift_key: Option<&'static str>,
    pub code: Option<&'static str>,
    pub text: Option<&'static str>,
    pub shift_text: Option<&'static str>,
    pub location: i64,
}

/// Resolved description after applying modifier state.
#[derive(Debug, Clone)]
pub struct KeyDescription {
    pub key_code: i64,
    pub key: String,
    pub text: String,
    pub code: String,
    pub location: i64,
}

macro_rules! key {
    ($kc:expr, $key:expr, $code:expr) => {
        KeyDefinition {
            key_code: $kc,
            shift_key_code: None,
            key: $key,
            shift_key: None,
            code: Some($code),
            text: None,
            shift_text: None,
            location: 0,
        }
    };
    ($kc:expr, $key:expr, $code:expr, shift_key: $sk:expr) => {
        KeyDefinition {
            key_code: $kc,
            shift_key_code: None,
            key: $key,
            shift_key: Some($sk),
            code: Some($code),
            text: None,
            shift_text: None,
            location: 0,
        }
    };
    ($kc:expr, $key:expr, $code:expr, text: $t:expr) => {
        KeyDefinition {
            key_code: $kc,
            shift_key_code: None,
            key: $key,
            shift_key: None,
            code: Some($code),
            text: Some($t),
            shift_text: None,
            location: 0,
        }
    };
    ($kc:expr, $key:expr, $code:expr, location: $loc:expr) => {
        KeyDefinition {
            key_code: $kc,
            shift_key_code: None,
            key: $key,
            shift_key: None,
            code: Some($code),
            text: None,
            shift_text: None,
            location: $loc,
        }
    };
    ($kc:expr, $key:expr, $code:expr, shift_key: $sk:expr, location: $loc:expr) => {
        KeyDefinition {
            key_code: $kc,
            shift_key_code: None,
            key: $key,
            shift_key: Some($sk),
            code: Some($code),
            text: None,
            shift_text: None,
            location: $loc,
        }
    };
    ($kc:expr, $key:expr, $code:expr, text: $t:expr, location: $loc:expr) => {
        KeyDefinition {
            key_code: $kc,
            shift_key_code: None,
            key: $key,
            shift_key: None,
            code: Some($code),
            text: Some($t),
            shift_text: None,
            location: $loc,
        }
    };
    ($kc:expr, $skc:expr, $key:expr, $code:expr, shift_key: $sk:expr, location: $loc:expr) => {
        KeyDefinition {
            key_code: $kc,
            shift_key_code: Some($skc),
            key: $key,
            shift_key: Some($sk),
            code: Some($code),
            text: None,
            shift_text: None,
            location: $loc,
        }
    };
    ($kc:expr, $key:expr, $code:expr, shift_key: $sk:expr, shift_text: $st:expr, location: $loc:expr) => {
        KeyDefinition {
            key_code: $kc,
            shift_key_code: None,
            key: $key,
            shift_key: Some($sk),
            code: Some($code),
            text: None,
            shift_text: Some($st),
            location: $loc,
        }
    };
    (nocode $kc:expr, $key:expr) => {
        KeyDefinition {
            key_code: $kc,
            shift_key_code: None,
            key: $key,
            shift_key: None,
            code: None,
            text: None,
            shift_text: None,
            location: 0,
        }
    };
    (nocode $kc:expr, $key:expr, $code:expr) => {
        KeyDefinition {
            key_code: $kc,
            shift_key_code: None,
            key: $key,
            shift_key: None,
            code: Some($code),
            text: None,
            shift_text: None,
            location: 0,
        }
    };
    (nocode_loc $kc:expr, $key:expr, $code:expr, $loc:expr) => {
        KeyDefinition {
            key_code: $kc,
            shift_key_code: None,
            key: $key,
            shift_key: None,
            code: Some($code),
            text: None,
            shift_text: None,
            location: $loc,
        }
    };
    (numpad $kc:expr, $skc:expr, $key:expr, $code:expr, $sk:expr) => {
        KeyDefinition {
            key_code: $kc,
            shift_key_code: Some($skc),
            key: $key,
            shift_key: Some($sk),
            code: Some($code),
            text: None,
            shift_text: None,
            location: 3,
        }
    };
}

pub static KEY_DEFINITIONS: LazyLock<HashMap<&'static str, KeyDefinition>> = LazyLock::new(|| {
    let mut m = HashMap::new();
    // digits
    m.insert("0", key!(48, "0", "Digit0"));
    m.insert("1", key!(49, "1", "Digit1"));
    m.insert("2", key!(50, "2", "Digit2"));
    m.insert("3", key!(51, "3", "Digit3"));
    m.insert("4", key!(52, "4", "Digit4"));
    m.insert("5", key!(53, "5", "Digit5"));
    m.insert("6", key!(54, "6", "Digit6"));
    m.insert("7", key!(55, "7", "Digit7"));
    m.insert("8", key!(56, "8", "Digit8"));
    m.insert("9", key!(57, "9", "Digit9"));
    // special
    m.insert(
        "Power",
        KeyDefinition {
            key_code: 0,
            shift_key_code: None,
            key: "Power",
            shift_key: None,
            code: Some("Power"),
            text: None,
            shift_text: None,
            location: 0,
        },
    );
    m.insert(
        "Eject",
        KeyDefinition {
            key_code: 0,
            shift_key_code: None,
            key: "Eject",
            shift_key: None,
            code: Some("Eject"),
            text: None,
            shift_text: None,
            location: 0,
        },
    );
    m.insert("Abort", key!(3, "Cancel", "Abort"));
    m.insert("Help", key!(6, "Help", "Help"));
    m.insert("Backspace", key!(8, "Backspace", "Backspace"));
    m.insert("Tab", key!(9, "Tab", "Tab"));
    m.insert("Numpad5", key!(numpad 12, 101, "Clear", "Numpad5", "5"));
    m.insert(
        "NumpadEnter",
        key!(13, "Enter", "NumpadEnter", text: "\r", location: 3),
    );
    m.insert("Enter", key!(13, "Enter", "Enter", text: "\r"));
    m.insert("\r", key!(13, "Enter", "Enter", text: "\r"));
    m.insert("\n", key!(13, "Enter", "Enter", text: "\r"));
    m.insert("ShiftLeft", key!(16, "Shift", "ShiftLeft", location: 1));
    m.insert("ShiftRight", key!(16, "Shift", "ShiftRight", location: 2));
    m.insert(
        "ControlLeft",
        key!(17, "Control", "ControlLeft", location: 1),
    );
    m.insert(
        "ControlRight",
        key!(17, "Control", "ControlRight", location: 2),
    );
    m.insert("AltLeft", key!(18, "Alt", "AltLeft", location: 1));
    m.insert("AltRight", key!(18, "Alt", "AltRight", location: 2));
    m.insert("Pause", key!(19, "Pause", "Pause"));
    m.insert("CapsLock", key!(20, "CapsLock", "CapsLock"));
    m.insert("Escape", key!(27, "Escape", "Escape"));
    m.insert("Convert", key!(28, "Convert", "Convert"));
    m.insert("NonConvert", key!(29, "NonConvert", "NonConvert"));
    m.insert("Space", key!(32, " ", "Space"));
    m.insert("Numpad9", key!(numpad 33, 105, "PageUp", "Numpad9", "9"));
    m.insert("PageUp", key!(33, "PageUp", "PageUp"));
    m.insert("Numpad3", key!(numpad 34, 99, "PageDown", "Numpad3", "3"));
    m.insert("PageDown", key!(34, "PageDown", "PageDown"));
    m.insert("End", key!(35, "End", "End"));
    m.insert("Numpad1", key!(numpad 35, 97, "End", "Numpad1", "1"));
    m.insert("Home", key!(36, "Home", "Home"));
    m.insert("Numpad7", key!(numpad 36, 103, "Home", "Numpad7", "7"));
    m.insert("ArrowLeft", key!(37, "ArrowLeft", "ArrowLeft"));
    m.insert("Numpad4", key!(numpad 37, 100, "ArrowLeft", "Numpad4", "4"));
    m.insert("Numpad8", key!(numpad 38, 104, "ArrowUp", "Numpad8", "8"));
    m.insert("ArrowUp", key!(38, "ArrowUp", "ArrowUp"));
    m.insert("ArrowRight", key!(39, "ArrowRight", "ArrowRight"));
    m.insert(
        "Numpad6",
        key!(numpad 39, 102, "ArrowRight", "Numpad6", "6"),
    );
    m.insert("Numpad2", key!(numpad 40, 98, "ArrowDown", "Numpad2", "2"));
    m.insert("ArrowDown", key!(40, "ArrowDown", "ArrowDown"));
    m.insert("Select", key!(41, "Select", "Select"));
    m.insert("Open", key!(43, "Execute", "Open"));
    m.insert("PrintScreen", key!(44, "PrintScreen", "PrintScreen"));
    m.insert("Insert", key!(45, "Insert", "Insert"));
    m.insert("Numpad0", key!(numpad 45, 96, "Insert", "Numpad0", "0"));
    m.insert("Delete", key!(46, "Delete", "Delete"));
    m.insert(
        "NumpadDecimal",
        KeyDefinition {
            key_code: 46,
            shift_key_code: Some(110),
            key: "\u{0000}",
            shift_key: Some("."),
            code: Some("NumpadDecimal"),
            text: None,
            shift_text: None,
            location: 3,
        },
    );
    // Digit keys with shift variants
    m.insert("Digit0", key!(48, "0", "Digit0", shift_key: ")"));
    m.insert("Digit1", key!(49, "1", "Digit1", shift_key: "!"));
    m.insert("Digit2", key!(50, "2", "Digit2", shift_key: "@"));
    m.insert("Digit3", key!(51, "3", "Digit3", shift_key: "#"));
    m.insert("Digit4", key!(52, "4", "Digit4", shift_key: "$"));
    m.insert("Digit5", key!(53, "5", "Digit5", shift_key: "%"));
    m.insert("Digit6", key!(54, "6", "Digit6", shift_key: "^"));
    m.insert("Digit7", key!(55, "7", "Digit7", shift_key: "&"));
    m.insert("Digit8", key!(56, "8", "Digit8", shift_key: "*"));
    m.insert("Digit9", key!(57, "9", "Digit9", shift_key: "("));
    // letter keys
    m.insert("KeyA", key!(65, "a", "KeyA", shift_key: "A"));
    m.insert("KeyB", key!(66, "b", "KeyB", shift_key: "B"));
    m.insert("KeyC", key!(67, "c", "KeyC", shift_key: "C"));
    m.insert("KeyD", key!(68, "d", "KeyD", shift_key: "D"));
    m.insert("KeyE", key!(69, "e", "KeyE", shift_key: "E"));
    m.insert("KeyF", key!(70, "f", "KeyF", shift_key: "F"));
    m.insert("KeyG", key!(71, "g", "KeyG", shift_key: "G"));
    m.insert("KeyH", key!(72, "h", "KeyH", shift_key: "H"));
    m.insert("KeyI", key!(73, "i", "KeyI", shift_key: "I"));
    m.insert("KeyJ", key!(74, "j", "KeyJ", shift_key: "J"));
    m.insert("KeyK", key!(75, "k", "KeyK", shift_key: "K"));
    m.insert("KeyL", key!(76, "l", "KeyL", shift_key: "L"));
    m.insert("KeyM", key!(77, "m", "KeyM", shift_key: "M"));
    m.insert("KeyN", key!(78, "n", "KeyN", shift_key: "N"));
    m.insert("KeyO", key!(79, "o", "KeyO", shift_key: "O"));
    m.insert("KeyP", key!(80, "p", "KeyP", shift_key: "P"));
    m.insert("KeyQ", key!(81, "q", "KeyQ", shift_key: "Q"));
    m.insert("KeyR", key!(82, "r", "KeyR", shift_key: "R"));
    m.insert("KeyS", key!(83, "s", "KeyS", shift_key: "S"));
    m.insert("KeyT", key!(84, "t", "KeyT", shift_key: "T"));
    m.insert("KeyU", key!(85, "u", "KeyU", shift_key: "U"));
    m.insert("KeyV", key!(86, "v", "KeyV", shift_key: "V"));
    m.insert("KeyW", key!(87, "w", "KeyW", shift_key: "W"));
    m.insert("KeyX", key!(88, "x", "KeyX", shift_key: "X"));
    m.insert("KeyY", key!(89, "y", "KeyY", shift_key: "Y"));
    m.insert("KeyZ", key!(90, "z", "KeyZ", shift_key: "Z"));
    // meta
    m.insert("MetaLeft", key!(91, "Meta", "MetaLeft", location: 1));
    m.insert("MetaRight", key!(92, "Meta", "MetaRight", location: 2));
    m.insert("ContextMenu", key!(93, "ContextMenu", "ContextMenu"));
    // numpad operators
    m.insert(
        "NumpadMultiply",
        key!(106, "*", "NumpadMultiply", location: 3),
    );
    m.insert("NumpadAdd", key!(107, "+", "NumpadAdd", location: 3));
    m.insert(
        "NumpadSubtract",
        key!(109, "-", "NumpadSubtract", location: 3),
    );
    m.insert("NumpadDivide", key!(111, "/", "NumpadDivide", location: 3));
    // function keys
    m.insert("F1", key!(112, "F1", "F1"));
    m.insert("F2", key!(113, "F2", "F2"));
    m.insert("F3", key!(114, "F3", "F3"));
    m.insert("F4", key!(115, "F4", "F4"));
    m.insert("F5", key!(116, "F5", "F5"));
    m.insert("F6", key!(117, "F6", "F6"));
    m.insert("F7", key!(118, "F7", "F7"));
    m.insert("F8", key!(119, "F8", "F8"));
    m.insert("F9", key!(120, "F9", "F9"));
    m.insert("F10", key!(121, "F10", "F10"));
    m.insert("F11", key!(122, "F11", "F11"));
    m.insert("F12", key!(123, "F12", "F12"));
    m.insert("F13", key!(124, "F13", "F13"));
    m.insert("F14", key!(125, "F14", "F14"));
    m.insert("F15", key!(126, "F15", "F15"));
    m.insert("F16", key!(127, "F16", "F16"));
    m.insert("F17", key!(128, "F17", "F17"));
    m.insert("F18", key!(129, "F18", "F18"));
    m.insert("F19", key!(130, "F19", "F19"));
    m.insert("F20", key!(131, "F20", "F20"));
    m.insert("F21", key!(132, "F21", "F21"));
    m.insert("F22", key!(133, "F22", "F22"));
    m.insert("F23", key!(134, "F23", "F23"));
    m.insert("F24", key!(135, "F24", "F24"));
    // lock keys
    m.insert("NumLock", key!(144, "NumLock", "NumLock"));
    m.insert("ScrollLock", key!(145, "ScrollLock", "ScrollLock"));
    // media
    m.insert(
        "AudioVolumeMute",
        key!(173, "AudioVolumeMute", "AudioVolumeMute"),
    );
    m.insert(
        "AudioVolumeDown",
        key!(174, "AudioVolumeDown", "AudioVolumeDown"),
    );
    m.insert("AudioVolumeUp", key!(175, "AudioVolumeUp", "AudioVolumeUp"));
    m.insert(
        "MediaTrackNext",
        key!(176, "MediaTrackNext", "MediaTrackNext"),
    );
    m.insert(
        "MediaTrackPrevious",
        key!(177, "MediaTrackPrevious", "MediaTrackPrevious"),
    );
    m.insert("MediaStop", key!(178, "MediaStop", "MediaStop"));
    m.insert(
        "MediaPlayPause",
        key!(179, "MediaPlayPause", "MediaPlayPause"),
    );
    // punctuation
    m.insert("Semicolon", key!(186, ";", "Semicolon", shift_key: ":"));
    m.insert("Equal", key!(187, "=", "Equal", shift_key: "+"));
    m.insert("NumpadEqual", key!(187, "=", "NumpadEqual", location: 3));
    m.insert("Comma", key!(188, ",", "Comma", shift_key: "<"));
    m.insert("Minus", key!(189, "-", "Minus", shift_key: "_"));
    m.insert("Period", key!(190, ".", "Period", shift_key: ">"));
    m.insert("Slash", key!(191, "/", "Slash", shift_key: "?"));
    m.insert("Backquote", key!(192, "`", "Backquote", shift_key: "~"));
    m.insert("BracketLeft", key!(219, "[", "BracketLeft", shift_key: "{"));
    m.insert("Backslash", key!(220, "\\", "Backslash", shift_key: "|"));
    m.insert(
        "BracketRight",
        key!(221, "]", "BracketRight", shift_key: "}"),
    );
    m.insert("Quote", key!(222, "'", "Quote", shift_key: "\""));
    m.insert("AltGraph", key!(225, "AltGraph", "AltGraph"));
    m.insert("Props", key!(247, "CrSel", "Props"));
    // aliases (key name = key value)
    m.insert("Cancel", key!(3, "Cancel", "Abort"));
    m.insert("Clear", key!(12, "Clear", "Numpad5", location: 3));
    m.insert("Shift", key!(16, "Shift", "ShiftLeft", location: 1));
    m.insert("Control", key!(17, "Control", "ControlLeft", location: 1));
    m.insert("Alt", key!(18, "Alt", "AltLeft", location: 1));
    m.insert("Accept", key!(nocode 30, "Accept"));
    m.insert("ModeChange", key!(nocode 31, "ModeChange"));
    m.insert(" ", key!(32, " ", "Space"));
    m.insert("Print", key!(nocode 42, "Print"));
    m.insert("Execute", key!(nocode 43, "Execute", "Open"));
    m.insert(
        "\u{0000}",
        key!(nocode_loc 46, "\u{0000}", "NumpadDecimal", 3),
    );
    // lowercase letters
    m.insert("a", key!(65, "a", "KeyA"));
    m.insert("b", key!(66, "b", "KeyB"));
    m.insert("c", key!(67, "c", "KeyC"));
    m.insert("d", key!(68, "d", "KeyD"));
    m.insert("e", key!(69, "e", "KeyE"));
    m.insert("f", key!(70, "f", "KeyF"));
    m.insert("g", key!(71, "g", "KeyG"));
    m.insert("h", key!(72, "h", "KeyH"));
    m.insert("i", key!(73, "i", "KeyI"));
    m.insert("j", key!(74, "j", "KeyJ"));
    m.insert("k", key!(75, "k", "KeyK"));
    m.insert("l", key!(76, "l", "KeyL"));
    m.insert("m", key!(77, "m", "KeyM"));
    m.insert("n", key!(78, "n", "KeyN"));
    m.insert("o", key!(79, "o", "KeyO"));
    m.insert("p", key!(80, "p", "KeyP"));
    m.insert("q", key!(81, "q", "KeyQ"));
    m.insert("r", key!(82, "r", "KeyR"));
    m.insert("s", key!(83, "s", "KeyS"));
    m.insert("t", key!(84, "t", "KeyT"));
    m.insert("u", key!(85, "u", "KeyU"));
    m.insert("v", key!(86, "v", "KeyV"));
    m.insert("w", key!(87, "w", "KeyW"));
    m.insert("x", key!(88, "x", "KeyX"));
    m.insert("y", key!(89, "y", "KeyY"));
    m.insert("z", key!(90, "z", "KeyZ"));
    m.insert("Meta", key!(91, "Meta", "MetaLeft", location: 1));
    // numpad-as-character
    m.insert("*", key!(106, "*", "NumpadMultiply", location: 3));
    m.insert("+", key!(107, "+", "NumpadAdd", location: 3));
    m.insert("-", key!(109, "-", "NumpadSubtract", location: 3));
    m.insert("/", key!(111, "/", "NumpadDivide", location: 3));
    // punctuation as character
    m.insert(";", key!(186, ";", "Semicolon"));
    m.insert("=", key!(187, "=", "Equal"));
    m.insert(",", key!(188, ",", "Comma"));
    m.insert(".", key!(190, ".", "Period"));
    m.insert("`", key!(192, "`", "Backquote"));
    m.insert("[", key!(219, "[", "BracketLeft"));
    m.insert("\\", key!(220, "\\", "Backslash"));
    m.insert("]", key!(221, "]", "BracketRight"));
    m.insert("'", key!(222, "'", "Quote"));
    // misc
    m.insert("Attn", key!(nocode 246, "Attn"));
    m.insert("CrSel", key!(nocode 247, "CrSel", "Props"));
    m.insert("ExSel", key!(nocode 248, "ExSel"));
    m.insert("EraseEof", key!(nocode 249, "EraseEof"));
    m.insert("Play", key!(nocode 250, "Play"));
    m.insert("ZoomOut", key!(nocode 251, "ZoomOut"));
    // shifted symbols
    m.insert(")", key!(48, ")", "Digit0"));
    m.insert("!", key!(49, "!", "Digit1"));
    m.insert("@", key!(50, "@", "Digit2"));
    m.insert("#", key!(51, "#", "Digit3"));
    m.insert("$", key!(52, "$", "Digit4"));
    m.insert("%", key!(53, "%", "Digit5"));
    m.insert("^", key!(54, "^", "Digit6"));
    m.insert("&", key!(55, "&", "Digit7"));
    m.insert("(", key!(57, "(", "Digit9"));
    // uppercase letters
    m.insert("A", key!(65, "A", "KeyA"));
    m.insert("B", key!(66, "B", "KeyB"));
    m.insert("C", key!(67, "C", "KeyC"));
    m.insert("D", key!(68, "D", "KeyD"));
    m.insert("E", key!(69, "E", "KeyE"));
    m.insert("F", key!(70, "F", "KeyF"));
    m.insert("G", key!(71, "G", "KeyG"));
    m.insert("H", key!(72, "H", "KeyH"));
    m.insert("I", key!(73, "I", "KeyI"));
    m.insert("J", key!(74, "J", "KeyJ"));
    m.insert("K", key!(75, "K", "KeyK"));
    m.insert("L", key!(76, "L", "KeyL"));
    m.insert("M", key!(77, "M", "KeyM"));
    m.insert("N", key!(78, "N", "KeyN"));
    m.insert("O", key!(79, "O", "KeyO"));
    m.insert("P", key!(80, "P", "KeyP"));
    m.insert("Q", key!(81, "Q", "KeyQ"));
    m.insert("R", key!(82, "R", "KeyR"));
    m.insert("S", key!(83, "S", "KeyS"));
    m.insert("T", key!(84, "T", "KeyT"));
    m.insert("U", key!(85, "U", "KeyU"));
    m.insert("V", key!(86, "V", "KeyV"));
    m.insert("W", key!(87, "W", "KeyW"));
    m.insert("X", key!(88, "X", "KeyX"));
    m.insert("Y", key!(89, "Y", "KeyY"));
    m.insert("Z", key!(90, "Z", "KeyZ"));
    // shifted punctuation
    m.insert(":", key!(186, ":", "Semicolon"));
    m.insert("<", key!(188, "<", "Comma"));
    m.insert("_", key!(189, "_", "Minus"));
    m.insert(">", key!(190, ">", "Period"));
    m.insert("?", key!(191, "?", "Slash"));
    m.insert("~", key!(192, "~", "Backquote"));
    m.insert("{", key!(219, "{", "BracketLeft"));
    m.insert("|", key!(220, "|", "Backslash"));
    m.insert("}", key!(221, "}", "BracketRight"));
    m.insert("\"", key!(222, "\"", "Quote"));
    // mobile keys
    m.insert("SoftLeft", key!(nocode_loc 0, "SoftLeft", "SoftLeft", 4));
    m.insert("SoftRight", key!(nocode_loc 0, "SoftRight", "SoftRight", 4));
    m.insert("Camera", key!(nocode_loc 44, "Camera", "Camera", 4));
    m.insert("Call", key!(nocode_loc 0, "Call", "Call", 4));
    m.insert("EndCall", key!(nocode_loc 95, "EndCall", "EndCall", 4));
    m.insert(
        "VolumeDown",
        key!(nocode_loc 182, "VolumeDown", "VolumeDown", 4),
    );
    m.insert("VolumeUp", key!(nocode_loc 183, "VolumeUp", "VolumeUp", 4));
    m
});

/// Modifier bitmask constants (matches Puppeteer & CDP).
pub const MODIFIER_ALT: i64 = 1;
pub const MODIFIER_CONTROL: i64 = 2;
pub const MODIFIER_META: i64 = 4;
pub const MODIFIER_SHIFT: i64 = 8;

/// Return the modifier bit for a key name, or 0 if not a modifier.
pub fn modifier_bit(key: &str) -> i64 {
    match key {
        "Alt" => MODIFIER_ALT,
        "Control" => MODIFIER_CONTROL,
        "Meta" => MODIFIER_META,
        "Shift" => MODIFIER_SHIFT,
        _ => 0,
    }
}

/// Resolve a key string into a full [`KeyDescription`], applying the current
/// modifier state (particularly shift).  Mirrors Puppeteer's
/// `_keyDescriptionForString`.
pub fn key_description_for_string(key_string: &str, modifiers: i64) -> Option<KeyDescription> {
    let def = KEY_DEFINITIONS.get(key_string)?;

    let shift = modifiers & MODIFIER_SHIFT != 0;

    // Start from definition defaults
    let mut key_code = def.key_code;
    let mut key = def.key.to_string();
    let code = def.code.unwrap_or("").to_string();
    let location = def.location;

    // Apply shift overrides
    if shift {
        if let Some(sk) = def.shift_key {
            key = sk.to_string();
        }
        if let Some(skc) = def.shift_key_code {
            key_code = skc;
        }
    }

    // Determine text: single-char keys generate text
    let mut text = if key.len() == 1 {
        key.clone()
    } else {
        String::new()
    };

    // Override text from definition
    if let Some(t) = def.text {
        text = t.to_string();
    }
    if shift {
        if let Some(st) = def.shift_text {
            text = st.to_string();
        }
    }

    // If non-shift modifiers are active, clear text (Puppeteer behavior)
    if modifiers & !MODIFIER_SHIFT != 0 {
        text = String::new();
    }

    Some(KeyDescription {
        key_code,
        key,
        text,
        code,
        location,
    })
}
