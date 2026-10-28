use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use sha2::{Digest, Sha256};
use thiserror::Error;

const TICKET_PREFIX: &str = "fxws_";
const TICKET_BYTES: usize = 32;
const TICKET_HEX_BYTES: usize = TICKET_BYTES * 2;
const TICKET_TTL: Duration = Duration::from_secs(30);
const MAX_PENDING_TICKETS: usize = 1024;
const HEX: &[u8; 16] = b"0123456789abcdef";

type CredentialVerifier = [u8; 32];
type TicketDigest = [u8; 32];

#[derive(Clone, Default)]
pub(crate) struct WebSocketTickets {
    pending: Arc<Mutex<HashMap<TicketDigest, PendingTicket>>>,
}

struct PendingTicket {
    credential_verifier: CredentialVerifier,
    expires_at: Instant,
}

impl WebSocketTickets {
    pub(crate) fn issue(
        &self,
        credential_verifier: CredentialVerifier,
    ) -> Result<String, TicketError> {
        let mut random = [0_u8; TICKET_BYTES];
        getrandom::fill(&mut random).map_err(|_| TicketError::Unavailable)?;
        let mut ticket = String::with_capacity(TICKET_PREFIX.len() + TICKET_HEX_BYTES);
        ticket.push_str(TICKET_PREFIX);
        for byte in random {
            ticket.push(char::from(HEX[usize::from(byte >> 4)]));
            ticket.push(char::from(HEX[usize::from(byte & 0x0f)]));
        }

        let digest: TicketDigest = Sha256::digest(ticket.as_bytes()).into();
        let mut pending = self.pending.lock().map_err(|_| TicketError::Unavailable)?;
        let now = Instant::now();
        pending.retain(|_, entry| entry.expires_at > now);
        if pending.len() >= MAX_PENDING_TICKETS || pending.contains_key(&digest) {
            return Err(TicketError::Unavailable);
        }
        pending.insert(
            digest,
            PendingTicket {
                credential_verifier,
                expires_at: now + TICKET_TTL,
            },
        );
        Ok(ticket)
    }

    pub(crate) fn consume(&self, ticket: &str) -> Result<CredentialVerifier, TicketError> {
        if ticket.len() != TICKET_PREFIX.len() + TICKET_HEX_BYTES
            || !ticket.starts_with(TICKET_PREFIX)
            || !ticket[TICKET_PREFIX.len()..]
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(TicketError::Unauthorized);
        }
        let digest: TicketDigest = Sha256::digest(ticket.as_bytes()).into();
        let mut pending = self.pending.lock().map_err(|_| TicketError::Unavailable)?;
        let entry = pending.remove(&digest).ok_or(TicketError::Unauthorized)?;
        if entry.expires_at <= Instant::now() {
            return Err(TicketError::Unauthorized);
        }
        Ok(entry.credential_verifier)
    }
}

#[derive(Debug, Error, Eq, PartialEq)]
pub(crate) enum TicketError {
    #[error("WebSocket ticket is invalid or expired")]
    Unauthorized,
    #[error("WebSocket ticket service is unavailable")]
    Unavailable,
}

#[cfg(test)]
mod tests {
    use super::{TicketError, WebSocketTickets};

    #[test]
    fn ticket_is_single_use_and_does_not_store_the_plaintext_value() {
        let tickets = WebSocketTickets::default();
        let verifier = [17; 32];
        let ticket = tickets.issue(verifier).expect("ticket issue succeeds");
        assert_eq!(ticket.len(), 69);
        assert_eq!(tickets.consume(&ticket), Ok(verifier));
        assert_eq!(tickets.consume(&ticket), Err(TicketError::Unauthorized));
    }

    #[test]
    fn malformed_tickets_are_rejected() {
        let tickets = WebSocketTickets::default();
        assert_eq!(
            tickets.consume("bearer-token"),
            Err(TicketError::Unauthorized)
        );
    }
}
