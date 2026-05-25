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
mod mailbox;
mod session;

pub use error::Error;
pub use jmap_client;
pub use mailbox::{SCREENER_MAILBOX_NAME, mailbox_id_by_name, mailbox_id_by_role};
pub use session::{Session, login_basic, login_bearer};
