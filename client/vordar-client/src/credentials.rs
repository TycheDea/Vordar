// Local account-credential storage (networking rework 1, finding 3): the
// real client mints a random 32-byte token once per character name and
// persists it in a small RON file (`name → hex token`) so every subsequent
// `Login` for that name presents the same credential — trust-on-first-use,
// the same shape a real password/registration flow lands on later.
//
// Deliberately a flat name → hex-string map, not a binary blob: it needs to
// be human-inspectable (a dev debugging "why was I denied?") and RON is
// already a workspace dependency.

use std::collections::HashMap;
use std::path::Path;
use vordar_protocol::AccountToken;

fn to_hex(token: &AccountToken) -> String {
    token.iter().map(|b| format!("{b:02x}")).collect()
}

fn from_hex(s: &str) -> Option<AccountToken> {
    if s.len() != 64 {
        return None;
    }
    let mut token = [0u8; 32];
    for (i, slot) in token.iter_mut().enumerate() {
        *slot = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(token)
}

/// Load `name`'s token from the RON map at `path`, minting and persisting a
/// fresh random one if the name isn't there yet (or the file doesn't exist,
/// or its entry for `name` is corrupt). Every call for the same `(path,
/// name)` pair returns the same token thereafter.
pub fn load_or_mint(path: &Path, name: &str) -> AccountToken {
    let mut map: HashMap<String, String> =
        std::fs::read_to_string(path).ok().and_then(|text| ron::from_str(&text).ok()).unwrap_or_default();

    if let Some(hex) = map.get(name) {
        match from_hex(hex) {
            Some(token) => return token,
            None => log::error!("credentials: '{name}' token in {path:?} is corrupt — minting a new one"),
        }
    }

    let mut token = [0u8; 32];
    getrandom::fill(&mut token).expect("getrandom failed to mint an account token");
    map.insert(name.to_owned(), to_hex(&token));
    match ron::to_string(&map) {
        Ok(text) => {
            if let Err(e) = std::fs::write(path, text) {
                log::error!("credentials: failed to persist {path:?}: {e}");
            }
        }
        Err(e) => log::error!("credentials: failed to encode {path:?}: {e}"),
    }
    token
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_path(tag: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!("vordar-credentials-test-{tag}-{}.ron", std::process::id()));
        let _ = std::fs::remove_file(&path);
        path
    }

    #[test]
    fn minted_token_is_returned_verbatim_on_second_call() {
        let path = temp_path("verbatim");
        let first = load_or_mint(&path, "alice");
        let second = load_or_mint(&path, "alice");
        assert_eq!(first, second, "the second call must reuse the minted token, not mint another");
    }

    #[test]
    fn distinct_names_get_distinct_tokens() {
        let path = temp_path("distinct");
        let alice = load_or_mint(&path, "alice");
        let bob = load_or_mint(&path, "bob");
        assert_ne!(alice, bob, "two names sharing one credentials file must not share a token");
        // Both survive together in the same file.
        assert_eq!(load_or_mint(&path, "alice"), alice);
        assert_eq!(load_or_mint(&path, "bob"), bob);
    }
}
