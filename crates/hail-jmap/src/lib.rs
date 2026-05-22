//! Thin wrapper over `jmap-client` with hail conventions.
//!
//! # Example
//!
//! ```no_run
//! use hail_jmap::login_basic;
//! use secrecy::SecretString;
//!
//! # async fn example() -> Result<(), hail_jmap::Error> {
//! let session = login_basic(
//!     "https://mail.example.org",
//!     "pat@example.org",
//!     SecretString::from("correct horse battery staple"),
//! )
//! .await?;
//!
//! println!("connected to JMAP account {}", session.account_id());
//! # Ok(())
//! # }
//! ```

mod error;
mod session;

pub use error::Error;
pub use jmap_client;
pub use session::{login_basic, login_bearer, Session};
