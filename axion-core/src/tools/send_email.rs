//! `send_email` tool — send an email via SMTP.
//!
//! Reads SMTP credentials from environment variables so no secret is ever
//! passed through the agent context.  Configure once at server start:
//!
//! ```text
//! AXION_SMTP_HOST=smtp.gmail.com
//! AXION_SMTP_PORT=587          # default
//! AXION_SMTP_USER=you@gmail.com
//! AXION_SMTP_PASS=app-password
//! AXION_SMTP_FROM=you@gmail.com
//! ```
//!
//! Works with any SMTP provider: Gmail (app password), SendGrid, Mailgun,
//! Amazon SES, Resend's SMTP bridge, Postmark, etc.
//!
//! The tool is intentionally omitted from role lists when SMTP env vars are
//! not set so agents never attempt to call it without credentials.

use lettre::{
    message::{header::ContentType, Mailbox},
    transport::smtp::authentication::Credentials,
    AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor,
};
use serde::Deserialize;
use std::str::FromStr;

#[derive(Deserialize)]
struct SendEmailArgs {
    /// Recipient email address.
    to: String,
    /// Email subject line.
    subject: String,
    /// Plain-text email body.
    body: String,
    /// Optional reply-to address.
    #[serde(default)]
    reply_to: Option<String>,
}

/// Returns true when all required SMTP env vars are present.
pub fn smtp_configured() -> bool {
    std::env::var("AXION_SMTP_HOST").is_ok()
        && std::env::var("AXION_SMTP_USER").is_ok()
        && std::env::var("AXION_SMTP_PASS").is_ok()
        && std::env::var("AXION_SMTP_FROM").is_ok()
}

pub async fn execute_send_email(arguments: &str) -> Result<String, String> {
    if !smtp_configured() {
        return Err(
            "send_email: SMTP is not configured. Set AXION_SMTP_HOST, AXION_SMTP_USER, \
             AXION_SMTP_PASS, and AXION_SMTP_FROM environment variables."
                .to_string(),
        );
    }

    let args: SendEmailArgs = serde_json::from_str(arguments)
        .map_err(|e| format!("send_email: invalid arguments: {e}"))?;

    let smtp_host = std::env::var("AXION_SMTP_HOST").unwrap();
    let smtp_port: u16 = std::env::var("AXION_SMTP_PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(587);
    let smtp_user = std::env::var("AXION_SMTP_USER").unwrap();
    let smtp_pass = std::env::var("AXION_SMTP_PASS").unwrap();
    let smtp_from = std::env::var("AXION_SMTP_FROM").unwrap();

    let from_mb = Mailbox::from_str(&smtp_from)
        .map_err(|e| format!("send_email: invalid AXION_SMTP_FROM address: {e}"))?;
    let to_mb = Mailbox::from_str(&args.to)
        .map_err(|e| format!("send_email: invalid 'to' address '{}': {e}", args.to))?;

    let mut builder = Message::builder()
        .from(from_mb)
        .to(to_mb)
        .subject(&args.subject)
        .header(ContentType::TEXT_PLAIN);

    if let Some(ref rt) = args.reply_to {
        let rt_mb = Mailbox::from_str(rt)
            .map_err(|e| format!("send_email: invalid reply_to address: {e}"))?;
        builder = builder.reply_to(rt_mb);
    }

    let message = builder
        .body(args.body)
        .map_err(|e| format!("send_email: failed to build message: {e}"))?;

    let creds = Credentials::new(smtp_user, smtp_pass);

    let mailer: AsyncSmtpTransport<Tokio1Executor> =
        AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&smtp_host)
            .map_err(|e| format!("send_email: SMTP relay error: {e}"))?
            .port(smtp_port)
            .credentials(creds)
            .build();

    mailer
        .send(message)
        .await
        .map_err(|e| format!("send_email: delivery failed: {e}"))?;

    Ok(format!(
        r#"{{"sent":true,"to":"{}","subject":"{}"}}"#,
        args.to, args.subject
    ))
}
