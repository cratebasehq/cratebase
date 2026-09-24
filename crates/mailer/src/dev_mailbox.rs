//! An in-memory ring buffer of mail captured by [`crate::LogBackend`] —
//! the "dev mail inbox" the server exposes at `/api/dev/mails` when no
//! real transport (SMTP/Resend) is configured, so verification/reset/OTP
//! links are clickable in local development with zero setup.
//!
//! Deliberately never touched by [`crate::SmtpBackend`] or
//! [`crate::ResendBackend`] — see [`crate::Mailer::dev_mailbox`], which
//! only returns `Some` when the wrapped backend is [`crate::LogBackend`].
//! Real outgoing mail is never captured here.

use std::collections::VecDeque;
use std::sync::Mutex;

use cratebase_core::DateTime;

use crate::message::{Address, Message};

/// How many recent emails a [`DevMailbox`] keeps before evicting the
/// oldest — enough to browse a dev session without unbounded growth.
pub const DEFAULT_CAPACITY: usize = 100;

/// One email captured by [`DevMailbox::capture`].
#[derive(Debug, Clone, PartialEq)]
pub struct CapturedMail {
    /// A fresh id per capture (see [`cratebase_core::record_id`]) — not
    /// derived from message content, so two identical emails still get
    /// distinct ids.
    pub id: String,
    pub to: Vec<Address>,
    pub from: Address,
    pub subject: String,
    pub html: String,
    pub text: Option<String>,
    pub sent_at: DateTime,
}

impl CapturedMail {
    fn from_message(msg: &Message) -> Self {
        CapturedMail {
            id: cratebase_core::record_id(),
            to: msg.to.clone(),
            from: msg.from.clone(),
            subject: msg.subject.clone(),
            html: msg.html.clone(),
            text: msg.text.clone(),
            sent_at: DateTime::now(),
        }
    }
}

/// A capped FIFO of the most recently captured mails. Cheap to share:
/// wrap in `Arc` and hand clones to both the mailer backend that writes
/// it and the server routes that read it.
#[derive(Debug)]
pub struct DevMailbox {
    capacity: usize,
    mails: Mutex<VecDeque<CapturedMail>>,
}

impl DevMailbox {
    /// A new, empty mailbox holding at most `capacity` mails (clamped to
    /// at least 1).
    pub fn new(capacity: usize) -> Self {
        DevMailbox {
            capacity: capacity.max(1),
            mails: Mutex::new(VecDeque::new()),
        }
    }

    /// Records `msg`, evicting the oldest entry first if already at
    /// capacity.
    pub fn capture(&self, msg: &Message) {
        let mail = CapturedMail::from_message(msg);
        let mut mails = self.mails.lock().unwrap_or_else(|e| e.into_inner());
        if mails.len() >= self.capacity {
            mails.pop_front();
        }
        mails.push_back(mail);
    }

    /// Every captured mail still held, newest first.
    pub fn list(&self) -> Vec<CapturedMail> {
        let mails = self.mails.lock().unwrap_or_else(|e| e.into_inner());
        mails.iter().rev().cloned().collect()
    }

    /// One captured mail by id, if it hasn't been evicted or cleared.
    pub fn get(&self, id: &str) -> Option<CapturedMail> {
        let mails = self.mails.lock().unwrap_or_else(|e| e.into_inner());
        mails.iter().find(|m| m.id == id).cloned()
    }

    /// Forgets everything captured so far.
    pub fn clear(&self) {
        self.mails.lock().unwrap_or_else(|e| e.into_inner()).clear();
    }

    pub fn len(&self) -> usize {
        self.mails.lock().unwrap_or_else(|e| e.into_inner()).len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl Default for DevMailbox {
    fn default() -> Self {
        DevMailbox::new(DEFAULT_CAPACITY)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(subject: &str) -> Message {
        Message::new(
            ("from@example.com".into(), String::new()),
            ("to@example.com".into(), String::new()),
            subject,
            "<p>hi</p>",
        )
    }

    #[test]
    fn starts_empty() {
        let mailbox = DevMailbox::new(10);
        assert!(mailbox.is_empty());
        assert_eq!(mailbox.list(), Vec::new());
    }

    #[test]
    fn lists_newest_first() {
        let mailbox = DevMailbox::new(10);
        mailbox.capture(&msg("First"));
        mailbox.capture(&msg("Second"));
        mailbox.capture(&msg("Third"));

        let subjects: Vec<String> = mailbox.list().into_iter().map(|m| m.subject).collect();
        assert_eq!(subjects, vec!["Third", "Second", "First"]);
    }

    #[test]
    fn evicts_the_oldest_once_over_capacity() {
        let mailbox = DevMailbox::new(3);
        for i in 0..5 {
            mailbox.capture(&msg(&format!("mail-{i}")));
        }
        assert_eq!(mailbox.len(), 3);
        let subjects: Vec<String> = mailbox.list().into_iter().map(|m| m.subject).collect();
        // Newest first; the two oldest (mail-0, mail-1) are gone.
        assert_eq!(subjects, vec!["mail-4", "mail-3", "mail-2"]);
    }

    #[test]
    fn get_finds_by_id_and_none_once_evicted() {
        let mailbox = DevMailbox::new(2);
        mailbox.capture(&msg("keep-me"));
        let id = mailbox.list()[0].id.clone();
        assert_eq!(mailbox.get(&id).map(|m| m.subject), Some("keep-me".to_string()));

        // Push it out of the ring.
        mailbox.capture(&msg("b"));
        mailbox.capture(&msg("c"));
        assert_eq!(mailbox.get(&id), None);
    }

    #[test]
    fn captures_full_content() {
        let mailbox = DevMailbox::new(10);
        let message = Message {
            to: vec![("a@example.com".into(), "A".into())],
            from: ("from@example.com".into(), "From".into()),
            subject: "Verify your email".into(),
            html: "<a href=\"https://example.com/verify?token=abc\">Verify</a>".into(),
            text: Some("Verify: https://example.com/verify?token=abc".into()),
            ..Default::default()
        };
        mailbox.capture(&message);
        let captured = &mailbox.list()[0];
        assert_eq!(captured.subject, "Verify your email");
        assert!(captured.html.contains("token=abc"));
        assert_eq!(
            captured.text.as_deref(),
            Some("Verify: https://example.com/verify?token=abc")
        );
        assert_eq!(captured.to, vec![("a@example.com".to_string(), "A".to_string())]);
        assert_eq!(
            captured.from,
            ("from@example.com".to_string(), "From".to_string())
        );
    }

    #[test]
    fn clear_empties_it() {
        let mailbox = DevMailbox::new(10);
        mailbox.capture(&msg("a"));
        mailbox.capture(&msg("b"));
        assert_eq!(mailbox.len(), 2);
        mailbox.clear();
        assert!(mailbox.is_empty());
    }
}
