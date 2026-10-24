//! Session accounting shared by FIM and Next Edit. A request reserves budget
//! before any HTTP is sent; the reservation is returned when the request is
//! cancelled before sending and kept (conservatively) when the provider omits
//! the real cost.
use std::{
    collections::{HashMap, VecDeque},
    time::{Duration, Instant},
};

use super::Settings;

const BACKOFF: Duration = Duration::from_secs(30);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ticket(u64);

#[derive(Clone, Debug, PartialEq)]
pub enum Refusal {
    /// HTTP 429 backoff or the per-minute window; retry at this instant.
    Cooldown(Instant),
    SessionRequests,
    SessionBudget,
}
impl Refusal {
    pub fn message(&self, feature: &str) -> String {
        match self {
            Refusal::Cooldown(_) => format!("{feature}: AI request rate limit; retry after cooldown"),
            Refusal::SessionRequests => {
                format!("{feature}: AI session request limit reached (shared by FIM and Next Edit); increase requests_per_session")
            }
            Refusal::SessionBudget => format!("{feature}: AI session spending budget reached (shared by FIM and Next Edit)"),
        }
    }
}

#[derive(Default)]
pub struct Ledger {
    /// Admission times inside the last minute, by ticket.
    recent: VecDeque<(u64, Instant)>,
    /// Reservation per unresolved ticket.
    in_flight: HashMap<u64, f64>,
    next: u64,
    cooldown: Option<Instant>,
    /// Requests that reached the provider.
    pub requests: u64,
    pub cost_usd: f64,
    /// In-flight reservations plus sent requests whose cost is unknown.
    pub reserved_usd: f64,
}

impl Ledger {
    fn reserve_amount(settings: &Settings) -> f64 {
        settings.reserve_per_request_usd.max(0.001)
    }

    pub fn cooldown(&self) -> Option<Instant> {
        self.cooldown.filter(|until| *until > Instant::now())
    }

    pub fn reserve(&mut self, settings: &Settings) -> Result<Ticket, Refusal> {
        let now = Instant::now();
        if let Some(until) = self.cooldown() {
            return Err(Refusal::Cooldown(until));
        }
        self.recent.retain(|(_, at)| now.duration_since(*at) < Duration::from_secs(60));
        if self.recent.len() >= settings.requests_per_minute.clamp(1, 60) {
            let oldest = self.recent.front().map_or(now, |(_, at)| *at);
            return Err(Refusal::Cooldown(oldest + Duration::from_secs(60)));
        }
        if self.requests + self.in_flight.len() as u64 >= settings.requests_per_session {
            return Err(Refusal::SessionRequests);
        }
        let amount = Self::reserve_amount(settings);
        if self.cost_usd + self.reserved_usd + amount > settings.session_budget_usd {
            return Err(Refusal::SessionBudget);
        }
        self.next += 1;
        self.recent.push_back((self.next, now));
        self.in_flight.insert(self.next, amount);
        self.reserved_usd += amount;
        Ok(Ticket(self.next))
    }

    /// The request never reached the provider: return quota and reservation.
    pub fn release(&mut self, ticket: Ticket) {
        if let Some(amount) = self.in_flight.remove(&ticket.0) {
            self.reserved_usd = (self.reserved_usd - amount).max(0.0);
            self.recent.retain(|(id, _)| *id != ticket.0);
        }
    }

    /// The request was sent (completed, failed or cancelled in flight). A known
    /// cost replaces the reservation; an unknown cost keeps it.
    pub fn settle(&mut self, ticket: Ticket, cost: Option<f64>) {
        let Some(amount) = self.in_flight.remove(&ticket.0) else { return };
        self.requests += 1;
        if let Some(cost) = cost {
            self.cost_usd += cost.max(0.0);
            self.reserved_usd = (self.reserved_usd - amount).max(0.0);
        }
    }

    /// Provider answered HTTP 429: both features back off together.
    pub fn backoff(&mut self) {
        self.cooldown = Some(Instant::now() + BACKOFF);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exhausted_budget_refuses_before_any_request() {
        let settings = Settings { session_budget_usd: 0.015, reserve_per_request_usd: 0.01, ..Default::default() };
        let mut ledger = Ledger::default();
        let first = ledger.reserve(&settings).unwrap_or_else(|e| panic!("reserve failed: {e:?}"));
        assert_eq!(ledger.reserve(&settings), Err(Refusal::SessionBudget));
        ledger.settle(first, None);
        assert_eq!(ledger.reserve(&settings), Err(Refusal::SessionBudget));
    }

    #[test]
    fn cancel_before_send_returns_reservation_and_quota() {
        let settings = Settings { requests_per_minute: 1, ..Default::default() };
        let mut ledger = Ledger::default();
        let ticket = ledger.reserve(&settings).unwrap_or_else(|e| panic!("reserve failed: {e:?}"));
        assert!(matches!(ledger.reserve(&settings), Err(Refusal::Cooldown(_))));
        ledger.release(ticket);
        assert_eq!(ledger.reserved_usd, 0.0);
        assert_eq!(ledger.requests, 0);
        assert!(ledger.reserve(&settings).is_ok());
    }

    #[test]
    fn sent_cancellation_is_counted_conservatively() {
        let settings = Settings::default();
        let mut ledger = Ledger::default();
        let ticket = ledger.reserve(&settings).unwrap_or_else(|e| panic!("reserve failed: {e:?}"));
        ledger.settle(ticket, None);
        assert_eq!(ledger.requests, 1);
        assert!(ledger.reserved_usd >= settings.reserve_per_request_usd);
        let ticket = ledger.reserve(&settings).unwrap_or_else(|e| panic!("reserve failed: {e:?}"));
        ledger.settle(ticket, Some(0.002));
        assert_eq!(ledger.requests, 2);
        assert!((ledger.cost_usd - 0.002).abs() < 1e-9);
        // Settling twice or releasing a settled ticket changes nothing.
        ledger.settle(ticket, Some(1.0));
        ledger.release(ticket);
        assert_eq!(ledger.requests, 2);
    }

    #[test]
    fn session_request_limit_counts_in_flight_requests() {
        let settings = Settings { requests_per_session: 1, ..Default::default() };
        let mut ledger = Ledger::default();
        let _ticket = ledger.reserve(&settings).unwrap_or_else(|e| panic!("reserve failed: {e:?}"));
        assert_eq!(ledger.reserve(&settings), Err(Refusal::SessionRequests));
    }

    #[test]
    fn backoff_is_shared() {
        let mut ledger = Ledger::default();
        ledger.backoff();
        assert!(matches!(ledger.reserve(&Settings::default()), Err(Refusal::Cooldown(_))));
    }
}
