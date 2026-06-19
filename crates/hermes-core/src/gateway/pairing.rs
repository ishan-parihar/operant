use rand::Rng;
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

const CODE_LEN: usize = 8;
const CODE_EXPIRY: Duration = Duration::from_secs(300);
const MAX_ATTEMPTS: u32 = 5;
const LOCKOUT_DURATION: Duration = Duration::from_secs(3600);

struct PairingCode {
    created_at: Instant,
}

pub struct PairingStore {
    codes: HashMap<String, PairingCode>,
    paired_users: HashSet<String>,
    failed_attempts: HashMap<String, (u32, Instant)>,
}

impl PairingStore {
    pub fn new() -> Self {
        let paired_users = Self::load_paired_users();
        Self {
            codes: HashMap::new(),
            paired_users,
            failed_attempts: HashMap::new(),
        }
    }

    pub fn generate_code(&mut self) -> String {
        self.codes
            .retain(|_, v| v.created_at.elapsed() < CODE_EXPIRY);
        let code: String = rand::thread_rng()
            .sample_iter(&rand::distributions::Alphanumeric)
            .take(CODE_LEN)
            .map(char::from)
            .collect();
        self.codes.insert(
            code.clone(),
            PairingCode {
                created_at: Instant::now(),
            },
        );
        code
    }

    pub fn validate_code(
        &mut self,
        user_id: &str,
        platform: &str,
        code: &str,
    ) -> Result<bool, String> {
        let key = format!("{}@{}", user_id, platform);
        if self.is_locked_out(user_id, platform) {
            return Err("Too many failed attempts. Try again later.".into());
        }
        if let Some(pc) = self.codes.remove(code) {
            if pc.created_at.elapsed() < CODE_EXPIRY {
                self.paired_users.insert(key);
                self.persist_paired_users();
                return Ok(true);
            }
        }
        let entry = self
            .failed_attempts
            .entry(key)
            .or_insert((0, Instant::now()));
        if entry.1.elapsed() > LOCKOUT_DURATION {
            *entry = (0, Instant::now());
        }
        entry.0 += 1;
        Ok(false)
    }

    pub fn is_paired(&self, user_id: &str, platform: &str) -> bool {
        self.paired_users
            .contains(&format!("{}@{}", user_id, platform))
    }

    pub fn is_locked_out(&self, user_id: &str, platform: &str) -> bool {
        let key = format!("{}@{}", user_id, platform);
        self.failed_attempts
            .get(&key)
            .map_or(false, |(count, first)| {
                *count >= MAX_ATTEMPTS && first.elapsed() < LOCKOUT_DURATION
            })
    }

    fn persist_paired_users(&self) {
        if let Some(dir) = dirs::home_dir().map(|h| h.join(".hermes")) {
            let _ = std::fs::create_dir_all(&dir);
            let users: Vec<&String> = self.paired_users.iter().collect();
            let _ = std::fs::write(
                dir.join("paired_users.json"),
                serde_json::to_string(&users).unwrap_or_default(),
            );
        }
    }

    fn load_paired_users() -> HashSet<String> {
        dirs::home_dir()
            .map(|h| h.join(".hermes/paired_users.json"))
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|s| serde_json::from_str::<Vec<String>>(&s).ok())
            .map(|v| v.into_iter().collect())
            .unwrap_or_default()
    }
}
