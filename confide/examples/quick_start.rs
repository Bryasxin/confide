use confide::confide;
use insta::assert_debug_snapshot;
use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Deserialize, Serialize)]
#[confide]
pub struct MyConfig {
    #[confide(default = 8080)]
    pub port: u16,

    #[confide(default_duration = "30s")]
    pub timeout: Duration,

    #[confide(default_bytes = "1 MiB")]
    pub buffer_size: u64,

    #[confide(default)]
    pub mode: String,

    #[confide(default = "127.0.0.1".to_string(), secret)]
    pub bind_address: String,
}

fn main() {
    let config = MyConfig::default();

    // Validate default values
    assert_eq!(config.port, 8080);
    assert_eq!(config.timeout, Duration::from_secs(30));
    assert_eq!(config.buffer_size, 1024 * 1024);
    assert_eq!(config.mode, String::default());
    assert_eq!(config.bind_address, "127.0.0.1".to_string());

    // Validate debug output
    assert_debug_snapshot!(config);
}
