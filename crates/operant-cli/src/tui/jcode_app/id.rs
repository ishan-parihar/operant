// Vendored from jcode (crates/jcode-core/src/id.rs), MIT License,
// Copyright (c) 2025 Jeremy Huang. Ported verbatim @ 0a9dc7805.
// Included: SESSION_NAMES (:57), session_icon (:201). `server_icon`/
// SERVER_MODIFIERS not ported (unreferenced by the ported tree), same for
// the id/name generators (App-bound). Callers were sedded from
// `crate::id::session_icon` to `crate::tui::jcode_app::id::session_icon`.
// See jcode_app/mod.rs for scope.

const SESSION_NAMES: &[(&str, &str)] = &[
    // Animals, nature companions, and client entities. Every emoji here is a single, widely-supported
    // codepoint (Unicode <= 12.0, no ZWJ sequences) with *default emoji
    // presentation* (no VS16 / U+FE0F needed). Text-default codepoints that rely
    // on VS16 render as monochrome outlines or tofu in macOS window titles
    // (Ghostty/Terminal tab and titlebar fonts ignore the selector), so they are
    // banned by `session_icons_render_as_single_safe_glyphs`.
    ("ant", "🐜"),
    ("bat", "🦇"),
    ("bird", "🐦"),
    ("bug", "🐛"),
    ("cat", "🐱"),
    ("chicken", "🐔"),
    ("chick", "🐥"),
    ("chipmunk", "🌰"),
    ("cow", "🐄"),
    ("crocodile", "🐊"),
    ("cricket", "🦗"),
    ("dog", "🐕"),
    ("dove", "🤍"),
    ("eagle", "🦅"),
    ("fish", "🐟"),
    ("fox", "🦊"),
    ("giraffe", "🦒"),
    ("hamster", "🐹"),
    ("ladybug", "🐞"),
    ("lobster", "🦞"),
    ("mosquito", "🦟"),
    ("owl", "🦉"),
    ("ox", "🐂"),
    ("pig", "🐷"),
    ("rat", "🐀"),
    ("ram", "🐏"),
    ("rooster", "🐓"),
    ("shrimp", "🦐"),
    ("sauropod", "🦕"),
    ("blowfish", "🐡"),
    ("buffalo", "🐃"),
    ("butterfly", "🦋"),
    ("badger", "🦡"),
    ("bear", "🐻"),
    ("crab", "🦀"),
    ("deer", "🦌"),
    ("duck", "🦆"),
    ("frog", "🐸"),
    ("goat", "🐐"),
    ("lion", "🦁"),
    ("wolf", "🐺"),
    ("horse", "🐴"),
    ("koala", "🐨"),
    ("llama", "🦙"),
    ("mouse", "🐭"),
    ("otter", "🦦"),
    ("panda", "🐼"),
    ("peacock", "🦚"),
    ("penguin", "🐧"),
    ("shark", "🦈"),
    ("sheep", "🐑"),
    ("sloth", "🦥"),
    ("snail", "🐌"),
    ("snake", "🐍"),
    ("spider", "🧶"),
    ("squid", "🦑"),
    ("swan", "🦢"),
    ("t-rex", "🦖"),
    ("tiger", "🐯"),
    ("turkey", "🦃"),
    ("whale", "🐋"),
    ("turtle", "🐢"),
    ("rabbit", "🐰"),
    ("parrot", "🦜"),
    ("jaguar", "🐆"),
    ("lizard", "🦎"),
    ("monkey", "🐒"),
    ("gorilla", "🦍"),
    ("orangutan", "🦧"),
    ("camel", "🐫"),
    ("elephant", "🐘"),
    ("rhino", "🦏"),
    ("hippo", "🦛"),
    ("boar", "🐗"),
    ("unicorn", "🦄"),
    ("kangaroo", "🦘"),
    ("hedgehog", "🦔"),
    ("skunk", "🦨"),
    ("raccoon", "🦝"),
    ("flamingo", "🦩"),
    ("dolphin", "🐬"),
    ("octopus", "🐙"),
    ("scorpion", "🦂"),
    ("zebra", "🦓"),
    ("stallion", "🐎"),
    ("dromedary", "🐪"),
    ("hog", "🐖"),
    ("kitten", "🐈"),
    ("poodle", "🐩"),
    ("hare", "🐇"),
    ("vole", "🐁"),
    ("dragon", "🐉"),
    ("humpback", "🐳"),
    ("guppy", "🐠"),
    ("nautilus", "🐚"),
    ("hatchling", "🐣"),
    ("wyvern", "🐲"),
    ("calf", "🐮"),
    ("macaque", "🐵"),
    ("tigress", "🐅"),
    // Additional terminal-safe identities. These deliberately stay on Unicode
    // 12 or older so they work in terminal tabs and window titles without a
    // bundled emoji font. `bee` is intentionally absent: 🐝 is reserved for the
    // global swarm marker rather than an individual client.
    ("puppy", "🐶"),
    ("duckling", "🐤"),
    ("mizaru", "🙈"),
    ("kikazaru", "🙉"),
    ("iwazaru", "🙊"),
    ("retriever", "🦮"),
    ("pawprint", "🐾"),
    ("piglet", "🐽"),
    ("bonehound", "🦴"),
    ("sabertooth", "🦷"),
    ("microbe", "🦠"),
    ("mushroom", "🍄"),
    ("cactus", "🌵"),
    ("clover", "🍀"),
    ("sunflower", "🌻"),
    ("hibiscus", "🌺"),
    ("blossom", "🌸"),
    ("daisy", "🌼"),
    ("tulip", "🌷"),
    ("rose", "🌹"),
    ("maple", "🍁"),
    ("seedling", "🌱"),
    ("evergreen", "🌲"),
    ("palmtree", "🌴"),
    ("herb", "🌿"),
];

/// Get an emoji icon for a session/client name word.
pub fn session_icon(name: &str) -> &'static str {
    SESSION_NAMES
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, icon)| *icon)
        .unwrap_or("💫")
}

/// Try to extract the memorable name from a session ID
/// e.g., "session_fox_1234567890_deadbeefcafebabe" -> Some("fox")
/// Verbatim from crates/jcode-core/src/id.rs:287.
pub fn extract_session_name(session_id: &str) -> Option<&str> {
    if let Some(rest) = session_id.strip_prefix("session_") {
        // Session names are the first token after the prefix.
        // This supports both old IDs (session_name_ts) and new IDs
        // with an added random suffix (session_name_ts_rand).
        if let Some(pos) = rest.find('_') {
            return Some(&rest[..pos]);
        }
    }
    None
}
