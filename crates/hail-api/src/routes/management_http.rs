//! Shared helpers for bounded Stalwart management HTTP calls.

use std::sync::LazyLock;
use std::time::Duration;

const MANAGEMENT_CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
const MANAGEMENT_REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

static MANAGEMENT_HTTP_CLIENT: LazyLock<reqwest::Client> = LazyLock::new(|| {
    reqwest::Client::builder()
        .connect_timeout(MANAGEMENT_CONNECT_TIMEOUT)
        .timeout(MANAGEMENT_REQUEST_TIMEOUT)
        .build()
        .expect("management HTTP client configuration is valid")
});

pub fn client() -> &'static reqwest::Client {
    &MANAGEMENT_HTTP_CLIENT
}
