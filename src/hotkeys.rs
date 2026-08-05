use global_hotkey::hotkey::{Code, HotKey, Modifiers};

pub fn parse_hotkey(spec: &str) -> Option<HotKey> {
    let mut modifiers = Modifiers::empty();
    let mut key: Option<Code> = None;

    for part in spec.split('+') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        match part.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => modifiers |= Modifiers::CONTROL,
            "shift" => modifiers |= Modifiers::SHIFT,
            "alt" => modifiers |= Modifiers::ALT,
            "super" | "meta" | "win" => modifiers |= Modifiers::SUPER,
            other => {
                key = code_from_str(other);
            }
        }
    }

    let code = key?;
    let mods = if modifiers.is_empty() {
        None
    } else {
        Some(modifiers)
    };
    Some(HotKey::new(mods, code))
}

fn code_from_str(s: &str) -> Option<Code> {
    Some(match s.to_ascii_uppercase().as_str() {
        "F1" => Code::F1,
        "F2" => Code::F2,
        "F3" => Code::F3,
        "F4" => Code::F4,
        "F5" => Code::F5,
        "F6" => Code::F6,
        "F7" => Code::F7,
        "F8" => Code::F8,
        "F9" => Code::F9,
        "F10" => Code::F10,
        "F11" => Code::F11,
        "F12" => Code::F12,
        _ => return None,
    })
}
