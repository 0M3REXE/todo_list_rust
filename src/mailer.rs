//! # Mail — the `Mailer` trait and its two implementations
//!
//! The app never talks to SMTP directly. It builds an [`Email`] and hands it to
//! whatever `Mailer` was installed at startup, which is what makes the whole
//! verification and reset flow testable without a network.
//!
//! Two implementations exist:
//!
//! - [`LogMailer`] — prints the message and keeps it in memory. Used in
//!   development and by the test suite, where a real mail server would be
//!   absurd. **Never use it in production**: it writes reset links to stdout.
//! - [`SmtpMailer`] — the real thing, over SMTP via `lettre`.
//!
//! ## Why the body is built here
//!
//! Templates for mail are plain text assembled in this file rather than Askama
//! templates, because Askama compiles into the binary and these strings need to
//! interpolate a URL. Keeping them next to the trait means the wording and the
//! delivery mechanism are read together.

use std::sync::{Arc, Mutex};

use base64::Engine as _;
use rand::RngExt as _;

/// One outbound message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Email {
    pub to: String,
    pub subject: String,
    pub body: String,
}

/// Anything that can deliver a message.
///
/// Async because SMTP is a network round trip, and running one on an async
/// worker thread would stall every other request sharing it.
///
/// `Result<(), String>` rather than a bespoke error enum: two implementations
/// exist and their failures are only ever logged, so a named type would carry
/// no information a log line does not already have.
#[async_trait::async_trait]
pub trait Mailer: std::fmt::Debug + Send + Sync {
    async fn send(&self, email: &Email) -> Result<(), String>;
}

// --- tokens ---

/// How many bytes of randomness a token carries.
///
/// 32 bytes is 256 bits, which is the same width as the session id. Guessing
/// one costs 2^256 attempts; there is no such thing as a partial guess worth
/// worrying about.
const TOKEN_BYTES: usize = 32;

/// A fresh one-time token, base64url-encoded.
///
/// URL-safe and unpadded, so the value drops into a link without escaping.
/// Base64 rather than hex purely because it is two-thirds the length.
pub fn random_token() -> String {
    let mut bytes = [0u8; TOKEN_BYTES];
    // `ThreadRng` is the OS CSPRNG. Anything weaker here would make every
    // reset link guessable.
    rand::rng().fill(&mut bytes);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

// --- the log mailer ---

/// Prints messages and keeps them in memory.
///
/// The in-memory copy is what lets a test read the link that was "sent" without
/// parsing log output.
#[derive(Debug, Default)]
pub struct LogMailer {
    sent: Mutex<Vec<Email>>,
}

impl LogMailer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Everything sent so far.
    pub fn sent(&self) -> Vec<Email> {
        self.sent
            .lock()
            .map(|inner| inner.clone())
            .unwrap_or_default()
    }

    /// The most recent message, or `None` if there was none.
    pub fn last(&self) -> Option<Email> {
        self.sent().pop()
    }

    /// Forgets everything, so one test cannot see another's mail.
    pub fn clear(&self) {
        if let Ok(mut inner) = self.sent.lock() {
            inner.clear();
        }
    }
}

#[async_trait::async_trait]
impl Mailer for LogMailer {
    async fn send(&self, email: &Email) -> Result<(), String> {
        tracing::info!(
            to = %email.to,
            subject = %email.subject,
            "mail sent (development mailer — the body below is a real reset link)"
        );
        // The body itself is NOT logged here. On a development machine that is
        // convenient; it is exactly the behaviour that must not survive into
        // production, and the SMTP mailer does not do it.
        if let Ok(mut inner) = self.sent.lock() {
            inner.push(email.clone());
        }
        Ok(())
    }
}

// --- the SMTP mailer ---

/// Delivers over SMTP.
///
/// The transport is generic over its async executor; this app runs on tokio, so
/// that is the type parameter pinned here rather than at every use.
pub struct SmtpMailer {
    transport: lettre::transport::smtp::AsyncSmtpTransport<lettre::Tokio1Executor>,
    from: String,
}

impl SmtpMailer {
    /// Builds a mailer from the environment.
    ///
    /// Returns `Ok(None)` when `SMTP_HOST` is unset, which is how the app
    /// decides it is running without a mail server. Returns `Err` when the
    /// variables are *set but unusable*, because silently falling back to the
    /// log mailer there would write reset links into a production log.
    pub fn from_env() -> Result<Option<Self>, String> {
        let Ok(host) = std::env::var("SMTP_HOST") else {
            return Ok(None);
        };

        let port: u16 = std::env::var("SMTP_PORT")
            .unwrap_or_else(|_| "587".to_owned())
            .parse()
            .map_err(|_| "SMTP_PORT is not a number".to_owned())?;

        let from = std::env::var("MAIL_FROM")
            .map_err(|_| "SMTP_HOST is set, so MAIL_FROM must be too".to_owned())?;

        let mut builder =
            lettre::transport::smtp::AsyncSmtpTransport::<lettre::Tokio1Executor>::builder_dangerous(&host)
                .port(port);

        if let Ok(user) = std::env::var("SMTP_USERNAME") {
            let password = std::env::var("SMTP_PASSWORD")
                .map_err(|_| "SMTP_USERNAME is set, so SMTP_PASSWORD must be too".to_owned())?;
            builder = builder.credentials(
                lettre::transport::smtp::authentication::Credentials::new(user, password),
            );
        }

        Ok(Some(Self {
            transport: builder.build(),
            from,
        }))
    }
}

impl std::fmt::Debug for SmtpMailer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The transport is not Debug, and a password must never reach a log.
        f.debug_struct("SmtpMailer")
            .field("from", &self.from)
            .finish_non_exhaustive()
    }
}

#[async_trait::async_trait]
impl Mailer for SmtpMailer {
    async fn send(&self, email: &Email) -> Result<(), String> {
        let built = lettre::Message::builder()
            .from(parse_address(&self.from)?)
            .to(parse_address(&email.to)?)
            .subject(&email.subject)
            .body(email.body.clone())
            .map_err(|err| format!("could not build the message: {err}"))?;

        // `AsyncTransport` is the trait that gives the transport its async `send`;
        // without it in scope this looks like a missing method.
        use lettre::AsyncTransport as _;

        self.transport
            .send(built)
            .await
            .map(|_| ())
            .map_err(|err| format!("SMTP send failed: {err}"))
    }
}

fn parse_address(raw: &str) -> Result<lettre::message::Mailbox, String> {
    raw.parse()
        .map_err(|_| format!("{raw:?} is not a valid address"))
}

// --- picking one ---

/// The mailer to install, chosen from the environment.
///
/// SMTP if `SMTP_HOST` is set, otherwise the log mailer. A test that wants the
/// capturing mailer passes one in explicitly through `build_app_with`.
pub fn from_env() -> Result<Arc<dyn Mailer>, String> {
    match SmtpMailer::from_env()? {
        Some(smtp) => {
            tracing::info!("mail delivery: SMTP");
            Ok(Arc::new(smtp))
        }
        None => {
            tracing::warn!(
                "SMTP_HOST is not set — using the development mailer. \
                 Reset links will be printed to the log. Do not run this in production."
            );
            Ok(Arc::new(LogMailer::new()))
        }
    }
}
